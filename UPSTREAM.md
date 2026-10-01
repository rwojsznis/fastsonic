# Syncing with upstream Fastpotify

Fastsonic is a fork of [crmne/fastpotify](https://github.com/crmne/fastpotify)
that replaced Spotify with Subsonic/OpenSubsonic and removed librespot, Spotify
Connect, devices and podcasts. Upstream still moves, and its backend-agnostic
fixes are worth taking.

## How the histories relate

This fork replayed upstream history by rebase rather than merge, so upstream's
commits are here under different hashes. Content-wise the fork is exactly
upstream 0.5.0-rc2 plus the Fastsonic work: `091e0a8` and upstream's `29fe2d0`
have identical trees.

Because the hashes differed, git computed a merge base 382 upstream commits
back and every sync conflicted with the whole rework. A `-s ours` merge of
`29fe2d0` fixed that once, without changing a byte of the tree. Keep it that
way: **merge sync branches into `main`, never squash or rebase them**, or the
merge base stops advancing and the next sync re-fights every resolved conflict.

## Reading the log after the graft

Recording `29fe2d0` as an ancestor made upstream's own copies of the replayed
commits reachable, so a plain `git log` shows everything before 0.5.0-rc2
twice: once as upstream wrote it, once as this fork rebased it. Nothing is
duplicated in the tree, only in the graph.

Use `--first-parent` for the fork's own line of development:

```sh
git log --first-parent --oneline
```

## Per-clone setup

```sh
git remote add upstream https://github.com/crmne/fastpotify.git
git config merge.ours.driver true    # activates .gitattributes 'merge=ours'
git config rerere.enabled true       # replays resolutions on the next sync
git config merge.conflictstyle zdiff3

# upstream's tags belong to upstream, and this fork numbers itself in the
# same `v*` space — so keep them apart:
git config remote.upstream.tagOpt --no-tags
git config --add remote.upstream.fetch "+refs/tags/*:refs/tags/upstream/*"
```

`merge.ours.driver` cannot live in the tree, so without it the `merge=ours`
entries in `.gitattributes` are inert and you resolve prose and packaging by
hand.

The driver applies to every three-way merge, not only to upstream syncs. A
`cherry-pick` or `rebase` of this fork's own commits that touch README.md or
`docs/**` keeps `main`'s copy of those files without a conflict, silently
dropping the commit's documentation. Move the fork's own commits with a real
text merge instead, and compare `git show --stat` before and after:

```sh
git -c merge.ours.driver="git merge-file --marker-size=%L %A %O %B" cherry-pick <commit>
```

## Tags

Upstream's release points arrive as `upstream/v0.6.0`, not `v0.6.0`. Without
the two tag settings above they land in the bare `v*` space, and because the
graft made upstream's commits ancestors of `main`, every one of them looks
like a tag on this fork's own history — `v0.6.0` pointing at "Update the Nix
vendor hash for 0.6.0", written by upstream's author, in the middle of this
fork's log. Then the fork cannot tag its own 0.6.0 without deleting
upstream's, and the next upstream release collides again.

The fork's own releases are the bare `v*` tags, and they are the only tags
pushed to `origin`. Sync to `upstream/v<version>`:

```sh
git merge upstream/v0.6.0
```

A clone that fetched upstream's tags before setting this up has them in the
bare space, mixed in with the fork's own. Do not sort them by hand — both
remotes hold the authority, so drop the local copies and fetch them back
into their two namespaces:

```sh
git tag -d $(git tag | grep -v '^upstream/')
git fetch upstream        # upstream's, under upstream/
git fetch origin --tags   # the fork's own, bare
```

## Doing a sync

```sh
git fetch upstream
git switch -c upstream-sync-<version> main
git merge <upstream waypoint>
# resolve, then:
cargo test
gh pr create --base main
```

Merge to upstream's release points — `upstream/v<version>` — rather than
jumping straight to `upstream/main`. Upstream sometimes lands a change and then reverts it, and a
single jump silently skips both sides of the resolution.

## What to skip

Resolve these to ours and move on. They stay recorded as merged, so they do not
come back:

- Nix vendor hash refreshes and website version publishing.
- Upstream's maintainer process docs and Copilot instructions.
- Anything Spotify-specific: librespot audio keys, Connect devices, Spotify
  deep links, the shared-application quota.

### Declined once, and why

These were looked at properly and turned down. A later sync will not offer
them again, so wanting one means cherry-picking it deliberately.

- **Optimistic song mutations** (`fa912c6`) and **duplicate-addition
  confirmation** (`930480d`). Worth having, but written on Spotify concepts:
  a recording key built from ISRC and `linked_from` for market relinking, and
  saved-state that branches on a `spotify:playlist:` prefix. Subsonic serves
  one recording under one id, so porting these is its own piece of work.
- **Resuming large playlists from cached progress** (`c052fe8`). `getPlaylist`
  returns every entry in one response; there is no partial fetch to resume.
- **Playlist folders** (`a3b8878`, `af022c2`) and **invitation edit grants**
  (`009309b`). Both ride on Spotify's rootlist, which went with `player.rs`.
- **Spotify links** (`594cc76`). No web address or URL scheme to open.
- **Removing clicks from explicit track changes** (`4c72bf3`). Wanted, and the
  fade itself is backend-agnostic: `Envelope` and `TransitionSource` are plain
  rodio. What does not carry over is the half that drives them. `AudioControl`
  exists because librespot's decoder keeps writing the old track after a skip,
  so the sink gates writes until the player says the replacement is loaded and
  reads a reset flag on its next `write`. This fork's engine owns both sides
  and calls `Output::restart` directly, so the port belongs in
  `src/engine/output.rs`, inline, and has its own question to answer: whether
  the worker may block the ~10 ms the fade needs to drain, and what to do when
  the sink is paused and so never drains at all. Its own piece of work.
- **Timing out stalled connection setup** (`800db1f`). A deadline on librespot
  session setup, carried by a patch to the librespot fork.
- **AccessKit's macOS adapter** (part of the 0.7.0 accessibility work). The
  accessibility itself is taken in full: `egui` carries the `accesskit` crate
  unconditionally, so every name, state, action and focus behaviour is here
  and its tests run. What is off on macOS is the one thing `eframe`'s
  `accesskit` feature actually adds, `egui-winit/accesskit`, which is the
  bridge to the platform screen reader. `accesskit_macos` puts a view in the
  window, and closing that window mid-session leaves AppKit's Touch Bar
  observation registered against it, so the next display flush aborts. The
  Winamp switch closes the window and starts a fresh event loop, so
  Cmd+Shift+M killed the app every time, reproducibly. Upstream 0.7.0 ships
  the same pair and has the same crash; see the comment in `Cargo.toml`.
  VoiceOver support here waits on an eframe that accepts `accesskit_macos`
  0.27.
- **The idle repaint interval** (part of `2221a30`). Upstream slows repaints to
  the API poll interval while local playback is idle, chosen through
  `Target::Local`. There is no remote target here and nothing to poll, so
  there is no second interval to pick.

What is worth taking is the backend-agnostic work: UI fixes, window and
platform behaviour, caching, fonts, and optimistic-update correctness.

## Sync through upstream 0.7.1

`upstream/v0.7.1` is recorded as the second parent of the Fastsonic sync
commit, so every original upstream commit remains in this fork's graph.

Taken and adapted to Fastsonic:

- Context menus on Home and Search cards.
- Shuffle Play keeps a filtered view but lets the engine shuffle an otherwise
  sorted collection.
- The queue names and links to the album, playlist, artist, or Liked Songs it
  is playing from.
- Flatpak tray registration uses the D-Bus connection's unique name.
- Play and Pause use a sample-clocked output fade after the visualizer tap.
- **Add to queue** names the FIFO behavior accurately, and the add-to-playlist
  submenu filters as the user types.

Already satisfied by Fastsonic's architecture:

- Clearing manual queue rows cannot remove the context's copy: the engine
  stores those as separate entries and its queue tests enforce the split.
- Shuffle needs no API recheck or stale-response defense: the engine changes
  and publishes its own queue before answering the command.
- Native media controls already receive artwork from Fastsonic's authenticated
  disk cache.

Declined because the corresponding product was removed: Spotify playback
authorization, Connect device discovery, credential-protection prose, and the
Spotify issue-triage and maintainer workflows. Upstream's website, packaging,
release metadata, and version remain fork-owned as usual.

## Sync through upstream 0.8.0

`upstream/v0.8.0` is recorded as the second parent of the Fastsonic sync
commit. The portable changes taken before that waypoint are:

- Long right-to-left titles are truncated to their actual shaped width.
- Yi artist names can use an installed system font, with a fixture-backed test.
- Shift-modified shortcuts no longer trigger their unmodified counterparts.
- Modified arrow keys stay with a focused text field for caret navigation.
- Selected song rows use a neutral fill and no focus outline, leaving the
  accent color to identify playback.

Upstream's Spotify personal-app authentication, session and cache work,
Spotifast rename, Spotify-specific history and playlist behavior, website,
packaging, update installer, release automation, translations, Omarchy
integration, and version metadata remain excluded. Larger portable features
whose implementations are coupled to upstream's rewritten Spotify app state
(finite collection scrolling, settings search, window restoration, and
Windows middle-button autoscroll) need separate Fastsonic-native ports.

## Sync through upstream 0.11.2

`upstream/v0.11.2` is recorded as the second parent of the Fastsonic sync
commit. The 0.9.0 merge, and the fork's 0.10 and 0.11 releases after it, had
left no record here, and this sync also audited what earlier merges marked as
merged: the 0.8.0 and 0.9.0 merges had recorded about fifty portable upstream
commits without taking them, and the 0.6.0 merge one. Those are taken now and
listed first. The early syncs (0.5.0 to 0.7.1) were checked commit by commit
and had dropped nothing else.

Recovered from the 0.6.0, 0.8.0 and 0.9.0 ranges:

- Library: persistent sort choices (`475a2d1`), Liked Songs that moves among
  the pins (`426ed0c`), a lit row for what is playing (`40e9ca6`),
  double-click to play a row (`27c6fab`), missing card menus (`78de9a8`),
  Refresh in a playlist's More menu (`6f1c4ea`), edge scrolling while dragging
  (`c374028`) and loading transitions with softened covers (`9f5a273`,
  `d1f093a`).
- Playlists: dragging the playing song (`8049ccc`), dropping songs between an
  open playlist's rows (`bac3fcd`), dragging a whole selection (`87d29eb`),
  arrow keys that stay on song rows (`f2a591d`), refreshes that wait for
  pending edits (`1e7beb8`), caches checked against the song count
  (`05473ec`), and cache files streamed through a buffer (`a07a8a5`,
  `b4de066`) sharing downloaded artwork (`edef8f0`).
- Queue and playback: albums queued as their songs and repeated songs kept
  in a batch (`5b8c1a4`, `8ea2b99`), filtered collections that play as their
  matching songs (`6ede8af`), Recent rows that play their named song and keep
  repeated short plays (`d0f8c4d`, `1c1586b`), and `fastsonic like` on Linux
  (`8eab887`).
- Windows and macOS windows: taskbar transport buttons (`fab4aa1`), a mini
  player without a taskbar button (`23bbb78`), middle-click autoscroll
  (`2ecbb4d`), the tray raising the window (`5eb054e`), maximized windows and
  the mini player's position restored safely (`119e7a5`, `09d7f17`,
  `a2c541f`) and a demo kept out of real window state (`20b7619`).
- Linux: always-on-top explained on Wayland (`2e9cc74`) and a named audio
  stream (`116105f`).
- Interface: Search kept clear of the update badge (`0bea123`, `1831928`, net),
  hideable Home shelves (`23c3a20`), local theme palettes without Omarchy
  (`ee16697`, `3e0f66d`, `125f052`), the system theme in new profiles
  (`b50be6e`), text-field editing menus (`1be1d77`), aligned submenus
  (`4294e55`), fallback baselines and graphics diagnostics (`46dc449`,
  `518f138`), EP labels and malformed dates (`01beed5`, `7b0a6ff`), compact
  artist credits (`75d98ac`), the update badge padding (`b6fe7c7`), stale
  library pages ignored (`b243f89`), bounded page caches (`b8250c7`), and the
  native media controls' artwork downloaded when no view has it (`0800158`,
  which the 0.7.1 section above had taken as already satisfied).

Taken from 0.9.0 to 0.11.2: the rest of the work since the last record,
among it radio pages through `getSimilarSongs` (`057cbbe`, `7ea2b29`), X11
taskbar hiding (`2cc5e43`), Linux middle-click autoscroll as an option
(`80b2e0b`), reordering Playing next and dropping on the Queue button
(`28384eb`, as engine commands and queue rule 10), the Library grid with its
300-pixel covers and narrow-heading fix (`73463cf`, `5e7f96b`, `c9f60e1`),
palette names and the bundled palettes (`e1ee031`, `2e7c150`, `929a213`), the
desktop's text rendering (`84a295f`, ported from fastframe-text rather than
taken as a dependency), nested skin folders, the macOS Dock menu and
title-bar double-click, MPRIS volume, credential redaction in errors, the
optional custom Windows title bar, vsync where a hidden window cannot block,
and the fixes listed in each commit's own message, which names its upstream
commit.

Declined:

- The fastframe crates and the egui and winit forks they ride on (`2ffa478`,
  `9a9c2c6`, `3bdc4ee`, `8840cbe`, `cd0bf03`, `fe8e0d3`, `a792a5b`,
  `aca81b8`, `8646630`, `554a313`, `e3251d7`), and what only they make
  possible: colour emoji (`3458210`, `f92da57`) and the Hyprland paste fix
  (`9f294ad`), which repairs the winit fork's Wayland file drops that stock
  winit does not have.
- Translations and the language setting (`880f3dd`, `872f426`, `b8ea5ad`,
  `fd49fe0`, `21f94c2`, `eee78da`), Omarchy (`14ae8e2` and the Omarchy halves
  of `ee16697` and `b50be6e`) and proxy settings (`a50347b`, apart from its
  accessible sign-in button, and `7ec0f17`).
- Spotify: remote devices (`7d07fe9`), access points (`caef305`), podcasts
  and episodes (`add8df6`, `cffda94`, `03ea765`), the personal app
  (`040c6e6`), Google Cast (`5bb82bd`), and Spotify-order and play-count
  prose (`9a906a7`, `e3a615a`).
- Branding and distribution: the icon refresh (`a0f4403`, `4666bcb`,
  `f01ce11`), AppImage (`41f8699`, `85b01ea`), the Spotifast rename and
  repository policy (`06bc3cf`, `dbde4f4`), website, launch film, release
  notes and versions, packaging, Homebrew, AUR, Nix and CI changes, and the
  author credit and sibling-app mentions (`910bc50`, `7e2d6c5`, `028036d`).
- Already true here, or not applicable: running without an audio output
  (`99ce5b4`), incremental playlist checkpoints and row appends (`ecab818`,
  `4d73105`; Subsonic sends a playlist whole), cached sort keys (`c6e6e13`),
  the Date added column (`befa5e3`), the Go to song control (`d58fc2a`),
  naming the next song on Next (`eaabab4`), glyph-level right-to-left
  reordering fixes (`7c3031a`, `74a4bae`; the fork reorders text before
  shaping), whole-pixel Inter (`092a9a7`, which `84a295f` replaced), the
  tinted-page fade (`a8c70b4`; the fade is off on every page), the shelf
  scroll-bar pair that reverted itself (`9754bd3`, `d4c3f56`), and the
  fallback-font log (`0ce65a1`).


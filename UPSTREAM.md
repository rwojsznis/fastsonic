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


## Sync through upstream 0.12.0

`upstream/v0.12.0` is recorded as the second parent of the Fastsonic sync
commit. Most of the range is upstream moving its own code into the fastframe
crates and finishing the Spotifast rename; the portable work in it is small.

Taken and adapted to Fastsonic:

- The Add to playlist submenu stays open when its filter is clicked, and
  grows back once the filter is cleared (`d732b06`).
- Library, Queue and Lyrics give up width before the top bar's controls run
  together, and the window's minimum width makes room for an open Queue or
  Lyrics panel (`e18b1b2`; the fork's bar has one badge, not two).
- The audio device rests while playback is paused (`2f81a7f`), ported into
  the engine's own output: a queue filled while paused leaves it resting.
- Two touchpad fixes from the scrolling upstream moved into fastframe-scroll
  (`5b70e3b`), applied to the fork's own copy: gestures are timed on the
  input clock, and only a frame's first pass may end one or move the glide.
- `fastsonic now-playing | head` ends quietly instead of panicking
  (`a2be13d`), a Windows clippy fix in the MilkDrop window (`d90b38c`), and
  the Winamp guide's note that Play restarts a playing song (`579edbe`).

Already true here, or not applicable:

- Repeated moves in Playing next (`916cd66`): the engine moves one named song
  per command, with no clear-and-re-add and no stale server answer to allow.
- Loading reported as paused to the media controls (`8f50455`), the default
  output followed and a failed output reopened (`c52deb0`), and the tray
  quitting rather than hiding when no panel shows it (`64d73b7`): the fork's
  own modules already do these.
- The macOS socket-path limit fixed in `b7ff417`: the fork's single-instance
  channel is a loopback port, not a socket.

Declined:

- The fastframe crates and the egui and winit forks: audio, scrolling,
  single instance, now playing, tray and shell (`c52deb0`, `5b70e3b`,
  `eead127`, `8f50455`, `4722ca1`, `cf880e1`, `af5477e`, `64d73b7`,
  `b7ff417`, `acf1f37`, `6e16b06`). `eead127` also replaced upstream's fixed,
  unauthenticated loopback control port with a user-private socket, or a
  token on Windows. The fork still has that port (47114) on macOS and
  Windows; closing it is its own piece of work, not a dependency swap.
- The Spotifast rename and the end of Fastpotify profile migration
  (`16a9dde`, `e10a8d4`, `d646a29`, `fa7ec94`), Turkish (`1f03a76`), and
  repository policy (`065ca22`, `d201d05`, `5f9c0d5`, `6b7a236`, `dce6a76`,
  `5138017`, `c036788`).
- Spotify: personal-app reauthorization (`4f42fe5`) and volume for remote
  devices (`059969c`).
- Release notes, website and version (`d6b45a7`, `5c66dde`, `ed9cb55`).

## Sync through upstream 0.13.0

`upstream/v0.13.0` is recorded as the second parent of the Fastsonic sync
commit. Every portable commit in the range was ported in its own fork
commit, whose message names the upstream commit and how it was adapted.

Taken and adapted to Fastsonic:

- Playback: the resampler's vector sums (`031e8c5`); a clean equalizer,
  limiter and visualiser on explicit track changes and seeks, sliders that
  answer a press, a 30 ms volume glide and 50 ms transport fades (`995c768`);
  the output fed when it has room instead of polled (`277a4fa`); playback
  that survives a changed or stalled output (`13876a8`, and the behaviour of
  `051d86d`), which also fixed the fork's own wrong speed after a reopen at
  another sample rate. The transport spinner (`4ba8d98`) was already true;
  only its test came across.
- Library and pages: pause on the playing context's cards and covers, and
  resume instead of restart (`556eda6`, `bc44757`); song radio named after
  its song (`6d970ac`); multi-song drags within a playlist (`a3e95f8`);
  green toggles under the pointer (`d692a22`, `ffd90cf`); album sort in
  track order (`5b7a5df`); Liked Songs honouring Colour from album art
  (`7f70ef1`); compact dialogs (`0c2560d`); the centred clear glyph
  (`09638fc`); the visualiser click setting (`224a0a1`); cheaper library
  sorting (`9d74afe`); the By You filter, by the Subsonic playlist owner
  (`25a8471`).
- Queue, search and command line: Remove from queue, as an engine command
  and queue rule 11 (`5553bdd`); one Recent search per run of typing
  (`4ebfcf0`); `fastsonic play-search`, through `search3` and, for playlists,
  their names (`7055320`).
- Artwork: WebP covers (`9d8902f`); covers asked for at the size they are
  drawn, through `getCoverArt`'s `size`, and scaled off the UI thread
  (`3566580`); cover colours in demo mode (`b85f17f`).
- Windows and platform: the Linux custom title bar, with GNOME's button
  layout read through the portal (`14db97c`, the behaviour of `7ebbf8d`);
  the Windows title bar's alignment (`d414667`); the mini player on a
  connected display (`54e2224`); closing a minimised window to the tray
  (`17f3a0d`); dual-GPU Macs (`12deee4`); from `5c3ae46` and `3d0b37b`,
  raising on niri, windows moved back from no monitor and the zoom kept in
  the settings; middle-click autoscroll (`cf5a802`, `13fedb9`); less motion
  (`04a3d26`); the MacBook notch card (`7eae8bb`, its device button and
  episodes left out).
- Power: the in-tree power module with its readers (`595ddc4`), playback
  out of App Nap (from `9d1ba8e`), paced visualisers and MilkDrop
  (`c07b5bf`), the tray asleep until something is due (`fea496e`, with the
  fork's own loop), the resume point and history kept safe (`f5a946c`),
  durable whole-file writes (the behaviour of `6c1465b`) and the daily
  update check and media-control covers held on metered connections
  (`73daaef`).
- Distribution: Windows on ARM with MilkDrop, built natively, and the ARM
  installer prompt and OpenGL message (`5d77caf`, `d0a1c5a`); the
  uninstaller's guard against a running copy, with its test in CI
  (`ef46278`); the login answer no longer `Debug` (`e9854af`'s point, applied
  to Navidrome's `/auth/login` answer). The rustls update (`68c453e`)
  arrived first through Dependabot.

Already true here, or not applicable:

- Liked Songs a page at a time (`6b5e8d9`): `getStarred2` answers whole.
- The tray icon's SVG filter (`36da0ff`): the fork's icon has none.
- `chacha20` (`de210c8`): not in the fork's lockfile.
- Signed update downloads (`1e37d0c`): the fork's update check only links
  to the release page and downloads nothing.

Declined:

- Spotify: adder names and Blends (`9dd23c9`, `cbe8176`), the login port
  (`98b37e7`, `f91ee83`), blank Web API playlists (`a9830c9`), Sonos and
  Google Cast (`60e112a`, `72192c8`), remote device polling and icons
  (`dd2d5e3`, `16c784c`, `58025c6`), artist top tracks (`d25a04b`), the
  cached library and Home behind the shared app (`778259e`), recording
  indexes (`ef49fba`), Web API logging, spinner and request scheduling
  (`412c187`, `ec6c2b6`, `3a7bee0`), Made for you (`f4b1539`), upcoming
  album pages (`fcf7baa`), playlist saves and owner avatars (`b27b240`;
  Subsonic counts no saves and Navidrome users have no pictures), the
  network-loss reconnection and offline views (`2ec2460`), and librespot
  (`3b89ea0`).
- The fastframe crates (`2b8f9f2`, `9132b28`, and the moves `9d1ba8e`,
  `051d86d`, `6c1465b`, `7ebbf8d`, `5c3ae46`, `3d0b37b` beyond the
  behaviours taken above).
- Translations (`eff77cd`, `88d5831`, `fc60826`).
- Packaging and distribution: the Windows Installer package and
  native-packages (`d72939f`, `63f8977`, `6d2ca90`, `0e87b95`, `cf2e27a`),
  the installers' copyright holder (`7048219`).
- Website, README, docs assets and repository policy (`457cb4a`, `34802e2`,
  `2590e33`, `92f0787`, `9e358bb`, `2010588`, `a6dc07b`, `6009eff`,
  `3813fa8`) and the release itself (`36964f1`).

## Sync through upstream 0.13.2

`upstream/v0.13.2` is recorded as the second parent of the Fastsonic sync
commit. Every portable commit in the range was ported in its own fork
commit, whose message names the upstream commit and how it was adapted.

Taken and adapted to Fastsonic:

- Saving: settings, the session and the history written off the UI thread
  by a file writer of the fork's own over `util::write_with` (`68160c5`,
  with `1735974`'s test flushes); a durable write on the UI thread refused,
  checked inside `util::write_with` itself (`4b2305f`); exit waiting 4 s
  and a suspend 1.5 s for pending writes (`71591cf`), and a flush that runs
  out of time logged (the behaviour of `cca1a82`); the rule in AGENTS.md
  (`e7ffe5f`).
- Stalls: an update of 250 ms or more logged, at most every ten seconds
  (`3fec863`, in the shape `8eef692` gave it, kept beside the fork's
  `Shell`).
- Visualisers and MilkDrop: a frame rate for the player bar and mini
  player visualisers (`96b1188`; Uncapped keeps the fork's full-rate
  request on Wayland, where vsync is off); Windows's 0 or 1 Hz hardware
  default ignored as a screen rate, and a frame rate learned from it
  forgotten (`82bc064`); the screen's rate asked of the window's monitor
  (`9270fc4`); presets listed off the UI thread (`c7d5dd8`).
- Interface: a `tint` palette colour and a header wash that scrolls with
  the page (`ecba317`); F for full-screen lyrics (`96827d4`), only where
  the expand button is, so not in the Winamp mini player or before
  signing in.
- Windows: the missing-OpenGL message on every build, pointing x64 at the
  getting-started guide (`01f7fb5`, `46ddcf3`); the main thread's window
  messages answered while the app waits in the tray, so Windows no longer
  calls it hung (fastframe 0.6.0's `99c53b6` and `d673207`, which upstream
  took in `8ec9775`), in `tray_native`'s idle.
- Flatpak: the Freedesktop 25.08 runtime (`8c4d87c`).

Already true here, or not applicable:

- Task Manager's name (`2ac8aad`): the fork's FileDescription already says
  Fastsonic.
- Local persistence for Windows grants (`bddf1cd`): the fork keeps its one
  credential in a file, not the Credential Manager.
- Cards taking the 640 pixel artwork (`173566d`): Spotify's renditions;
  covers here are asked for at their drawn size.

Declined:

- Spotify: genuine Cast devices (`6ddcceb`, `228002d`), playlist order read
  again on return (`e77ceb7`), the playback callback port (`fc9a8e2`) and
  retrying the saved playback grant (`bf9d916`).
- The fastframe crates (`cca1a82`, `8eef692`, `433bcd3`, and `8ec9775`
  beyond the tray fix taken above).
- Translations (`db1d60a`, `03a505a`, `7ec5de7`).
- The Windows Installer package's update leftovers (`38f05db`, `f833832`,
  `c6aabc7`): the fork ships no MSI and its update check downloads nothing.
- Repository policy (`18562a5`), the website (`f911ff7`, `0c9a989`) and
  the releases themselves (`30307eb`, `2c0f545`).

## Sync through upstream 0.13.3

`upstream/v0.13.3` is recorded as the second parent of the Fastsonic sync
commit. Every portable commit in the range was ported in its own fork
commit, whose message names the upstream commit and how it was adapted.

Taken and adapted to Fastsonic:

- MilkDrop: preset pack entries written only under a plain file name, so
  `C:x.milk` cannot leave the presets folder on Windows (`34aa7df`); the
  libprojectM length check on HLSL identifiers (`6a60c30`'s projectm-sys
  revision), which the fork's `respect-cmake-generator` branch now builds.
- Artwork refused by its declared length or as soon as it passes the
  limit, rather than once it is all in memory, and a stored zip entry
  held to the size it declares (`f70a58a`), in the fork's fetch, which
  sends the server's extra headers and checks the content type.

Declined:

- Spotify: the sign-in redirect listener's limits (`b60ea01`; Subsonic
  sign-in has no browser redirect) and the librespot fixes (`6a60c30`
  beyond projectM).
- The website (`677dbf6`) and the release itself (`a4c1b3e`).

## Sync through upstream 0.14.2

`upstream/v0.14.2` is recorded as the second parent of the Fastsonic sync
commit. Every portable commit in the range was ported in its own fork
commit, whose message names the upstream commit and how it was adapted.
Upstream moved its fastframe crates from 0.6.0 to 0.14.0 in the range, and
the fixes and features it adopted through them were ported in-tree.

Taken and adapted to Fastsonic:

- Playback: the repeat mode kept across restarts (`e8057e9`); Repeat One
  turned back into Repeat All by Next, a Previous that steps back or
  playing something else (`cb415c2`); the engine's thread raised above
  other work on every platform (`24e3b57`, fastframe `760e829`); the gaps
  the audio device played logged (`b772e08`, fastframe `cf01365`'s
  counting); the app's icon beside its stream in Linux volume mixers
  (`f3d47d9`, the icon only); the computer kept awake while music plays
  (`d995a36`, fastframe `ba9b89b`, `58fc5c3`), in the power module.
- Rows and the queue: the chosen copy of a repeated playlist song started
  and marked alone (`93dd3d2`); only the picked copy removed, through
  `updatePlaylist`'s `songIndexToRemove` (`bdabd61`); Play next in song and
  selection menus (`67f31e9`); a search song played on its own
  (`c3faf59`); the seed first in song radio (`f71df0a`); the Recent tab
  shared rather than copied (`08d88f4`); Home's top tiles following what
  was played from (`55d1b5f`, playlists, albums and Liked Songs, as the
  fork records them).
- Interface: Add to playlist from a liked song's heart (`dff96e4`); the
  header cover enlarged on a click (`7c0e77c`); skeleton loading pages
  (`ee8900d`); the visualizer changed from beside the player bar's words
  (`2bc03fd`); the visualizer swept across the cover's colours
  (`447a0ee`); a Release notes button in About (`3037975`); even padding in
  settings cards (`2526523`); a light and a dark theme for Follow system
  (`3428548`, fastframe `4edd14d`, without Omarchy); Han fallback faces
  for simplified Chinese and Japanese (fastframe `0ecccb4`, `c3187b5`);
  egui 0.36.2 from crates.io (`d54bf07`'s version, not the egui fork).
- Keys, lyrics and the mini player: single-key shortcuts with a song row
  focused (`2bf13b1`); Page Up, Page Down, Home and End (`da44f07`); no
  side panel room in full-screen lyrics (`703c0b1`); the queue column in
  full-screen lyrics, toggled by Q (`3737062`); the mini player's sliders
  moved only by their own track (`d44db71`); D stepping through 1x
  (`d69bb2e`); visualizers in shade mode (`5aec231`); your own equalizer
  presets (`952da3b`); LRCLIB's 429 waited out (`cffb506`); a byte order
  mark kept from eating the first synced line (`a910bfe`, applied to
  LRCLIB's synced lyrics, as the fork reads no `.lrc` files); the main
  window opened at its default size below its minimum (`7bb1d95`).
- Listening activity (`3a735af`), rebuilt on `getNowPlaying`: what other
  people on the same server are playing, polled every 15 seconds only
  while the panel is open.
- Desktop: starting with your computer, with the window or in the tray
  (`3dbb309`, with `77339ee`, `dc27a83` and fastframe `cee4b40`, `55652f5`),
  in-tree in `src/autostart`; the menu bar named on macOS (`9188c5e`);
  menu-bar only, without a Dock icon (`e3b8627`, fastframe `a415de7`); a
  white or black tray icon (`4115b0e`, fastframe `77b4274`); the menu-bar
  menu on any click (`9d1f7a6`, fastframe `1f3d7d7`); the menu-bar item
  rebuilt after a window's event loop ends, the symptom `dffb1be`'s winit
  patch fixes, and the Dock icon of an unbundled run (fastframe `97ceedd`);
  memory kept flat in the tray (`0cfb8c9`, fastframe `3ee0dbb`); a portable
  Windows copy's data next to it (`83e1546`, fastframe `08b1f6b`, sharing
  the one single-instance port rather than taking a slot of its own as
  fastframe `8cf7b6c` does); checking a Windows download (`1bd872b`).

Already true here, or not applicable:

- `c85c038`: the engine's queue is already measured in time.
- `d5e4e6b` (fastframe `e46fe7b`): cpal 0.16 opens the Windows default
  as a real endpoint; the virtual default device arrived in cpal 0.18.
- `118e26a` (fastframe `2b5ab56`): the libpulse backend is upstream's own,
  and cpal 0.16 has no device ids; the fork opens a saved output by name.
- Fastframe `e504c70` (the device already rests while paused), `ab6f5c8`
  and `e03f4e5` (the fork's callback allocates no conversion buffers),
  `1cccc82` (a suspend hook keep-awake does not need).
- `dcb43ad`: no Home shelf here waits on a seed. `e3b8b45`: the fork keeps
  no lasting unlike record, and `getStarred2` answers whole. `a8b8033`:
  every playlist entry is a playable row. `6aaf503`: the fork has one
  locale. `ad21363`: the account menu has one line. `7c177d1`: the sign-in
  card already grows to fit. `7807d21`: the fork's bidi tests have no
  glyph-counting helper.

Declined:

- The real-time output callback on Linux (fastframe `cf01365`, `b722c80`,
  upstream `e3d03db`, `a3ff025`): the fork plays through cpal's ALSA host,
  normally to `default`, `pipewire` or `pulse`, which `b722c80`'s own rule
  keeps out of real time.
- Following the desktop's text size (fastframe `55910e8`): upstream does
  not call it as of 0.14.2. Validated local fonts (fastframe `fcf20de`):
  an API for a font picker the fork does not have.
- Spotify: the playback-session rootlist, buddylist, Pathfinder and
  librespot work (`bda7b8c`, `97844ec`, `7d63f55`, `f9f2060`, `fade33e`,
  `cd93c5e`, `a0efaeb`, `56ec5ac`, `466bdf5`), podcast refresh (`d284f5a`),
  folders (`46a4ecf`), Your top mixes (`f0b5522`), the device picker's
  output and Cast ranking (`fa527b6`, `b96f805`), the Windows grant copy
  (`46b827a`; the fork keeps its credential in a file), play counts
  (`247c2ac`; Subsonic's `playCount` is your own, not a song's
  popularity), New releases from followed artists (`2e5142f`), hidden
  songs (`4322346`), a playlist's local files (`3e63cb4`, `88edc38`,
  `2784cbd`, `e86d3de`), the shared app, audio keys and their banners
  (`19b7443`, `8a4a6bc`, `791fab0`), Top Songs periods (`6911a79`, from
  `/me/top`), remote volume (`cd5ba1f`) and the personal-app sign-in
  (`f06b05b`, `7ed46cc`, `f7b3810`).
- The winit fork's AppKit patches (`dffb1be`, `0b3e980`); with stock
  winit the macOS hidden start opens a transparent first window that closes
  itself to the menu bar.
- The fastframe crates and their version moves (`d5e4e6b`, `77339ee`,
  `0cfb8c9`, `e3d03db`, `a3ff025`, `d54bf07`, `7c78448`, `118e26a` and
  `f3d47d9` beyond the behaviours taken above), with fastframe's cpal fork
  and PipeWire host (`6f87eb8`, `861716c`, `504962c`), notifications,
  audio input, the AppImage updater and its own release feed.
- AppImage (`a3282ba`, `c5f2639`, `c303ef6`), translations (`3e82631`,
  `35f4837`, `9775fc7`) and repository policy and triage (`dc23c1c`,
  `673aaed`, `cd9cd41`, `e5449a1`, `076aa3b`, `e53fa09`, `a31e35f`).
- The website (`4ba5a6a`, `1bc082f`, `ebc0341`, `5730dbc`) and the
  releases themselves (`b6c939d`, `c2e41ee`, `3ce8b3f`, `3bc6bb4`).

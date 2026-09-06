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

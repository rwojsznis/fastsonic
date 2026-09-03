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

## Per-clone setup

```sh
git remote add upstream https://github.com/crmne/fastpotify.git
git config merge.ours.driver true    # activates .gitattributes 'merge=ours'
git config rerere.enabled true       # replays resolutions on the next sync
git config merge.conflictstyle zdiff3
```

`merge.ours.driver` cannot live in the tree, so without it the `merge=ours`
entries in `.gitattributes` are inert and you resolve prose and packaging by
hand.

## Doing a sync

```sh
git fetch upstream
git switch -c upstream-sync-<version> main
git merge <upstream waypoint>
# resolve, then:
cargo test
gh pr create --base main
```

Merge to upstream's release points rather than jumping straight to
`upstream/main`. Upstream sometimes lands a change and then reverts it, and a
single jump silently skips both sides of the resolution.

## What to skip

Resolve these to ours and move on. They stay recorded as merged, so they do not
come back:

- Nix vendor hash refreshes and website version publishing.
- Upstream's maintainer process docs and Copilot instructions.
- Anything Spotify-specific: librespot audio keys, Connect devices, Spotify
  deep links, the shared-application quota.

What is worth taking is the backend-agnostic work: UI fixes, window and
platform behaviour, caching, fonts, and optimistic-update correctness.

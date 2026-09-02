# Fastsonic agent guide

Follow `CONTRIBUTING.md`; it is the canonical product and contribution policy.
These instructions add implementation constraints for coding agents.

## Product boundaries

- Keep Fastsonic a small native client for a music server you run yourself.
  Do not add a browser engine, telemetry, a hosted backend, or an account
  system. Nothing the app does should require anything but the user's own
  server.
- **Subsonic first.** Talk to the server over Subsonic/OpenSubsonic, which
  Navidrome, Gonic and others all speak. Reach for Navidrome's native
  `/auth/login` + `/api/*` only where Subsonic genuinely cannot answer, keep
  those calls in one module, and make the degraded path obvious when the
  server is not Navidrome.
- Audio is decoded in this process from a stream the server hands over. Do not
  add server-side playback (jukebox mode), offline sync, or a second source of
  audio.
- One server, one set of credentials. There is no shared application identity,
  no per-playlist capability, and no quota to work around.
- Do not broaden a task into adjacent features or a general refactor. Preserve
  existing user behaviour unless the task explicitly changes it.

**While the migration in `migration/` is in flight**, this repository is
still largely the Spotify client it was forked from. The boundaries above
describe what is being built, not what is here today. Read
`migration/PROGRESS.md` before starting: it says which phase the work is in,
what is claimed, and what is deliberately parked. Do not delete Spotify code
ahead of Phase 5 — the order is deliberate, so a working reference survives
for as long as possible.

## Architecture

- `src/ui/` draws views and emits `Action`s. Apply actions after drawing in
  `src/app.rs`; do not mutate application state from inside a borrowed view.
- Network and playback work belongs on the runtime in `src/backend.rs` or on
  the audio thread in `src/engine/`, never as blocking work on the UI thread.
- Keep platform integrations behind target-specific modules or `cfg` blocks.
  A fix for one platform must keep the other two targets compiling.
- Settings and state files must remain readable, backward compatible, and
  atomically written. Never log credentials or authorization responses.
- Prefer existing dependencies. Explain any new crate in `Cargo.toml` next to
  the dependency when the reason is not obvious.

Read `migration/04-auth-and-config.md` before changing authentication,
credential storage, or network behaviour; `docs/_reference/how-it-connects.md`
still describes the Spotify client and is rewritten at P6.1. Read
`docs/_reference/queue.md` before touching the queue: its rules are the
contract, and they are enforced by the tests in `src/engine/queue.rs` (what
plays next) and `src/app.rs` (what the panel draws and what a click asks
for). Read the nearby module tests before changing a state machine or API
fallback.

The interface is optimistic where the answer has to travel. A control
shows its result the moment it is used: a double-clicked song is the
playing song, a hearted song has its heart. The backend then makes it true;
an answer that still tells the story from before the user's action is
stale, so hold the shown state and ask again rather than let the lagging
answer undo what the user just did. Nothing the user did may ever flicker
away and come back.

The queue is the exception, and the reason is worth knowing: it lives in
the engine, one channel away rather than one network away, and the engine
answers a queue command before it opens a track or sends a request. Next,
Play next, Clear and playing a row are all published back in time for the
next frame, which the answer itself wakes — so the panel draws what the
engine says and guesses at nothing. Do not add a second copy of the queue to the interface to make it
feel faster; it cannot be, and a second copy is a second opinion.

Every visualiser, the spectrum analyser, the oscilloscope, and MilkDrop,
shows the signal post-equalizer and pre-volume: the EQ shapes what is
heard so the picture follows it, and the volume knob never moves the
picture. Zero volume still dances.

## Branches

Work on `main`. Commit there directly, one topic per commit, each
compiling and passing the checks on its own. Feature branches and pull
requests are for outside contributors; the maintainer's own work, and
work done with the maintainer, does not go through them.

## Definition of done

- Add focused regression tests for changed behaviour. Use the `demo` feature
  for deterministic UI coverage and screenshots.
- Update the README and docs when user-visible behaviour, settings, files, or
  network access changes.
- Run the full checks from `CONTRIBUTING.md`. Do not weaken a lint, delete a
  test, or add an `allow` merely to make CI green without explaining why the
  underlying rule does not apply.
- Report platform coverage honestly. Do not claim a platform was tested when
  it was only compiled or reasoned about.

## Releases

A release is not the tag alone. Do these in order:

1. Change the `Cargo.toml` version and update the lockfile with a build.
   Commit and push this before the tag so the binaries report the right
   version.
2. Push the `v*` tag, which triggers the release workflow. Wait for every
   required artifact and `checksums.txt`, then replace the generated notes
   with written ones.
3. A prerelease stops here. Keep the stable version current on the website.
   The prerelease remains available from GitHub's releases page.
4. For a stable release, only after the GitHub release exists, update
   `docs/_config.yml` `fastsonic_version` and
   `docs/_data/versions.yml`. The new version becomes `current` and points
   at `/download/`; the previous version keeps a link to its GitHub release.
   Never make the download page point at files that do not exist yet.

There is no Homebrew tap and no AUR package. Upstream had both; the fork
inherits neither until it actually publishes them, and no release step,
README, or download page may imply otherwise.

Skipping an applicable step ships a release that lies somewhere; the dropdown
was forgotten once already.

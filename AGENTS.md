# Fastpotify agent guide

Follow `CONTRIBUTING.md`; it is the canonical product and contribution policy.
These instructions add implementation constraints for coding agents.

## Product boundaries

- Keep Fastpotify a small native Spotify client. Do not add a browser engine,
  telemetry, a hosted backend, or alternate sources for Spotify audio.
- Playback capabilities come from librespot. Do not advertise or implement a
  capability merely because its name appears in a protobuf or enum. In
  particular, do not pursue Spotify Lossless or DRM circumvention unless
  lawful support first lands upstream.
- Do not broaden a task into adjacent features or a general refactor. Preserve
  existing user behaviour unless the task explicitly changes it.

## Architecture

- `src/ui/` draws views and emits `Action`s. Apply actions after drawing in
  `src/app.rs`; do not mutate application state from inside a borrowed view.
- Network and playback work belongs on the runtime in `src/backend.rs` or in
  the player engine in `src/player.rs`, never as blocking work on the UI
  thread.
- Keep platform integrations behind target-specific modules or `cfg` blocks.
  A fix for one platform must keep the other two targets compiling.
- Settings and state files must remain readable, backward compatible, and
  atomically written. Never log credentials or authorization responses.
- Prefer existing dependencies. Explain any new crate in `Cargo.toml` next to
  the dependency when the reason is not obvious.

Read `docs/_reference/how-it-connects.md` before changing authentication,
Spotify requests, Connect, credential storage, or network behaviour. Read the
nearby module tests before changing a state machine or API fallback.

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

# Contributing to Fastpotify

Fastpotify is deliberately small, native, and focused. A contribution is a
good fit when it makes the Spotify desktop experience better without turning
the project into a browser, a collection of fallbacks, or a second backend.

## Before opening an issue

Search the open and closed issues first. For a bug, use the bug form and
include the requested log and exact reproduction steps. A report that cannot
be reproduced and contains no useful diagnostics may be closed until that
information is available.

For a feature, explain the user problem rather than only naming a feature.
Large changes should be discussed in an issue before code is written. An
implementation is not, by itself, a reason for the project to accept a new
product direction.

Some boundaries come from Spotify or from upstream libraries:

- Local playback requires Spotify Premium because librespot requires it.
- Spotify Lossless is not available through librespot. Fastpotify will
  reconsider it if librespot gains lawful upstream support; proposals that
  depend on bypassing Spotify's DRM are out of scope.
- Spotify tracks must come from Spotify. Substituting audio from YouTube,
  Piped, `yt-dlp`, or another catalogue is out of scope.
- Fastpotify will not embed a browser engine, add telemetry, or introduce a
  Fastpotify-operated service.

Issues that are duplicates, outside these boundaries, or contain no actionable
problem may be closed with a short explanation. That is scope management, not
a judgement on the person who opened them.

## Design principles

1. **Native and fast.** Startup time, idle work, memory use, and binary size
   are product features. Keep the UI thread free of network and disk waits.
2. **Focused.** Prefer a complete, coherent workflow over a collection of
   settings, modes, and speculative features.
3. **Honest integrations.** Use Spotify's Web API and librespot for what they
   support. Do not scrape, impersonate capabilities, bypass technical
   protections, or silently replace one service with another.
4. **Cross-platform by default.** Linux, macOS, and Windows are supported
   products. Platform-specific code must be isolated and the other targets
   must keep compiling.
5. **Small dependency surface.** Reuse the standard library and existing
   crates where practical. A new dependency needs a concrete benefit worth
   its build time, binary size, maintenance, and security cost.
6. **Visible failure, private data.** Errors should be actionable, rate limits
   should be respected, and credentials must never appear in logs. Network
   behaviour belongs in the documentation.

## Pull requests

Keep each pull request to one coherent change. Explain why the change belongs
in Fastpotify, what behaviour changed, and how you verified it. Avoid unrelated
formatting, drive-by refactors, generated prose, and large mechanical rewrites.

Every pull request is held to the same standard whether it was written by a
person, generated with an AI tool, or both. The author is responsible for
understanding every line and for responding to review with specific technical
reasoning.

Code changes should include tests for behaviour that can regress. UI changes
should include before/after screenshots or a short recording and should use
demo mode where possible. User-visible behaviour, settings, files, or network
access must be documented in the same pull request.

Run the same checks CI runs before submitting:

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
cargo test --locked --all-features --doc
RUSTDOCFLAGS='-D warnings' cargo doc --locked --all-features --no-deps
(cd docs && bundle exec jekyll build)
```

Linux needs the development packages listed in the README; `nix develop`
provides the complete development environment. CI repeats the test suite on
Linux, macOS, and Windows. Passing CI is required, but does not replace review
for correctness, product fit, maintainability, or security.

By contributing, you agree that your contribution is licensed under the
project's MIT License.

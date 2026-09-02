# Copilot instructions for Fastsonic

Use `AGENTS.md` and `CONTRIBUTING.md` as the source of truth for every change
and review.

When reviewing a pull request, prioritize correctness, regressions, product
fit, cross-platform behaviour, UI-thread blocking, credential exposure, and
unnecessary dependencies. Treat violations of the documented product
boundaries as blockers. In particular, flag alternate sources for Spotify
audio, DRM circumvention, embedded browser engines, telemetry, and hosted
Fastsonic services.

Check that behavioural changes have focused tests and that user-visible
settings, files, and network access are documented. For UI changes, ask for
visual evidence when the pull request does not provide it. Do not spend review
comments on formatting that rustfmt already enforces.

CI passing is necessary but not proof that a change is correct. Give concrete,
actionable findings tied to changed lines; avoid generic summaries and do not
approve or recommend merging code that you cannot substantively evaluate.

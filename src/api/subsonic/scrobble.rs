//! When to tell the server what is being played.
//!
//! Navidrome knows nothing about your listening that a client did not tell
//! it. `getAlbumList2 type=recent` and `type=frequent` — the two shelves
//! Home is built from — and the play counts the native API sorts by are
//! *only* what has been scrobbled. Skip this and Home is permanently empty:
//! scrobbling and personalisation are one feature, not two.
//!
//! There are two reports, and they mean different things:
//!
//! - `submission=false` is **now playing**. It expires, so it is repeated
//!   while a song plays, and it is what other clients see in
//!   `getNowPlaying` under this app's name.
//! - `submission=true` is **the play itself**. It moves the play count, and
//!   it is sent once per play.
//!
//! When to send the second one is the Last.fm rule every Subsonic server
//! expects: not until the song has been listened to for half its length or
//! four minutes, whichever comes first, and never for anything under thirty
//! seconds. Listened, not seeked past — dragging the scrubber to the end
//! does not make a play, so this counts time spent playing rather than the
//! position reached.
//!
//! This type decides; it does not call. The engine drives it and sends what
//! it asks for, which is what makes the rule testable without a server.

use std::time::Duration;

/// Songs shorter than this are never submitted.
const MINIMUM_LENGTH: Duration = Duration::from_secs(30);
/// Listening for this long submits, however long the song is.
const ALWAYS_ENOUGH: Duration = Duration::from_secs(4 * 60);
/// How often a now-playing report is repeated while a song plays.
const NOW_PLAYING_EVERY: Duration = Duration::from_secs(30);

/// What the engine should send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Report {
    /// `scrobble?submission=false`.
    NowPlaying { id: String },
    /// `scrobble?submission=true&time=…`, where `time` is when the play
    /// *started*, in milliseconds since the epoch.
    Played { id: String, started_at_ms: u64 },
}

#[derive(Clone, Debug)]
struct Play {
    id: String,
    duration: Duration,
    started_at_ms: u64,
    /// Time actually spent playing, which is not the position: a listener
    /// who seeks to the last bar has not played the song.
    listened: Duration,
    /// When `listened` was last brought up to date, and the position it was
    /// at then, so ordinary playback can be measured between two updates.
    last_seen_ms: u64,
    last_position: Duration,
    was_playing: bool,
    announced_at_ms: Option<u64>,
    submitted: bool,
}

/// Follows one player and says what to report.
#[derive(Clone, Debug, Default)]
pub struct Scrobbler {
    current: Option<Play>,
}

impl Scrobbler {
    pub fn new() -> Self {
        Self::default()
    }

    /// The player's state, as often as it changes. `now_ms` is the wall
    /// clock in milliseconds since the epoch — the same clock `time=` wants.
    ///
    /// Returns what to send, which is usually nothing.
    pub fn observe(
        &mut self,
        song_id: Option<&str>,
        position: Duration,
        duration: Duration,
        playing: bool,
        now_ms: u64,
    ) -> Vec<Report> {
        let Some(song_id) = song_id.filter(|id| !id.is_empty()) else {
            self.current = None;
            return Vec::new();
        };

        let mut reports = Vec::new();
        let changed = self.current.as_ref().is_none_or(|play| play.id != song_id);
        if changed {
            self.current = Some(Play {
                id: song_id.to_string(),
                duration,
                started_at_ms: now_ms,
                listened: Duration::ZERO,
                last_seen_ms: now_ms,
                last_position: position,
                was_playing: playing,
                announced_at_ms: None,
                submitted: false,
            });
        }
        let Some(play) = self.current.as_mut() else {
            return reports;
        };
        if duration > Duration::ZERO {
            play.duration = duration;
        }

        // Count the time between two observations, but only as much of it as
        // the song moved forward by: a seek covers wall-clock time without
        // covering any listening, and a paused player covers neither.
        let elapsed = Duration::from_millis(now_ms.saturating_sub(play.last_seen_ms));
        if play.was_playing && playing {
            let advanced = position.saturating_sub(play.last_position);
            play.listened += advanced.min(elapsed);
        }
        play.last_seen_ms = now_ms;
        play.last_position = position;
        play.was_playing = playing;

        if playing {
            let due = play
                .announced_at_ms
                .is_none_or(|at| now_ms.saturating_sub(at) >= NOW_PLAYING_EVERY.as_millis() as u64);
            if due {
                play.announced_at_ms = Some(now_ms);
                reports.push(Report::NowPlaying {
                    id: play.id.clone(),
                });
            }
        }

        if !play.submitted && play.listened >= threshold(play.duration) {
            play.submitted = true;
            reports.push(Report::Played {
                id: play.id.clone(),
                started_at_ms: play.started_at_ms,
            });
        }
        reports
    }

    /// Playback stopped altogether. Nothing is reported: a play that had
    /// earned its submission already sent it, and one that had not does not
    /// earn it by ending.
    pub fn stopped(&mut self) {
        self.current = None;
    }

    /// Whether the current song has already been counted, so a caller can
    /// tell "not yet" from "nothing playing".
    pub fn submitted(&self) -> bool {
        self.current.as_ref().is_some_and(|play| play.submitted)
    }
}

/// How much listening a song of this length needs before it counts.
/// `None` is impossible: a song under thirty seconds gets a threshold no
/// amount of listening can reach.
fn threshold(duration: Duration) -> Duration {
    if duration < MINIMUM_LENGTH {
        return Duration::MAX;
    }
    (duration / 2).min(ALWAYS_ENOUGH)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SONG: &str = "s1";
    const THREE_MINUTES: Duration = Duration::from_secs(180);

    /// Plays `song` straight through in one-second steps, returning
    /// everything the scrobbler asked to send.
    fn play_through(scrobbler: &mut Scrobbler, duration: Duration, seconds: u64) -> Vec<Report> {
        let mut reports = Vec::new();
        for second in 0..=seconds {
            reports.extend(scrobbler.observe(
                Some(SONG),
                Duration::from_secs(second),
                duration,
                true,
                second * 1000,
            ));
        }
        reports
    }

    fn submissions(reports: &[Report]) -> usize {
        reports
            .iter()
            .filter(|report| matches!(report, Report::Played { .. }))
            .count()
    }

    /// Rule: the server is told a song started as soon as it starts,
    /// because that is what other clients see.
    #[test]
    fn a_song_is_announced_the_moment_it_starts() {
        let mut scrobbler = Scrobbler::new();
        let reports = scrobbler.observe(Some(SONG), Duration::ZERO, THREE_MINUTES, true, 0);
        assert_eq!(
            reports,
            vec![Report::NowPlaying {
                id: SONG.to_string()
            }]
        );
    }

    /// Rule: a paused player is not announced over and over.
    #[test]
    fn a_paused_song_is_not_announced() {
        let mut scrobbler = Scrobbler::new();
        scrobbler.observe(Some(SONG), Duration::ZERO, THREE_MINUTES, true, 0);
        let reports = scrobbler.observe(
            Some(SONG),
            Duration::from_secs(10),
            THREE_MINUTES,
            false,
            60_000,
        );
        assert!(reports.is_empty(), "{reports:?}");
    }

    /// Rule: the play counts at halfway, once, and not before.
    #[test]
    fn a_song_counts_at_halfway_and_only_once() {
        let mut scrobbler = Scrobbler::new();
        let early = play_through(&mut scrobbler, THREE_MINUTES, 89);
        assert_eq!(submissions(&early), 0, "89 seconds of three minutes");

        let rest = play_through(&mut scrobbler, THREE_MINUTES, 179);
        assert_eq!(submissions(&rest), 1);
        assert!(scrobbler.submitted());

        // Playing on to the end does not count it twice.
        let more = scrobbler.observe(
            Some(SONG),
            Duration::from_secs(180),
            THREE_MINUTES,
            true,
            180_000,
        );
        assert_eq!(submissions(&more), 0);
    }

    /// Rule: a long song counts after four minutes rather than at its own
    /// halfway point, which for an hour-long recording would be absurd.
    #[test]
    fn a_long_song_counts_after_four_minutes() {
        assert_eq!(threshold(Duration::from_secs(3600)), ALWAYS_ENOUGH);
        let mut scrobbler = Scrobbler::new();
        let reports = play_through(&mut scrobbler, Duration::from_secs(3600), 241);
        assert_eq!(submissions(&reports), 1);
    }

    /// Rule: something too short to be a song never counts, however long it
    /// is left running.
    #[test]
    fn a_very_short_song_never_counts() {
        let mut scrobbler = Scrobbler::new();
        let reports = play_through(&mut scrobbler, Duration::from_secs(9), 60);
        assert_eq!(submissions(&reports), 0);
    }

    /// Rule: seeking to the end is not listening. This is the case the
    /// whole "count time played, not position reached" design exists for.
    #[test]
    fn dragging_to_the_end_does_not_make_a_play() {
        let mut scrobbler = Scrobbler::new();
        scrobbler.observe(Some(SONG), Duration::ZERO, THREE_MINUTES, true, 0);
        let reports = scrobbler.observe(
            Some(SONG),
            Duration::from_secs(179),
            THREE_MINUTES,
            true,
            2_000,
        );
        assert_eq!(submissions(&reports), 0);
        assert!(!scrobbler.submitted());
    }

    /// Rule: neither is leaving a song paused for an hour.
    #[test]
    fn a_long_pause_is_not_listening() {
        let mut scrobbler = Scrobbler::new();
        scrobbler.observe(Some(SONG), Duration::ZERO, THREE_MINUTES, true, 0);
        scrobbler.observe(
            Some(SONG),
            Duration::from_secs(5),
            THREE_MINUTES,
            false,
            5_000,
        );
        let reports = scrobbler.observe(
            Some(SONG),
            Duration::from_secs(5),
            THREE_MINUTES,
            true,
            3_600_000,
        );
        assert_eq!(submissions(&reports), 0);
    }

    /// Rule: now-playing is repeated while a song plays, because the
    /// server's copy of it expires — but not on every state change, or a
    /// moved scrubber would be a burst of requests.
    #[test]
    fn now_playing_is_repeated_but_not_constantly() {
        let mut scrobbler = Scrobbler::new();
        let first_ten = play_through(&mut scrobbler, THREE_MINUTES, 10);
        assert_eq!(
            first_ten.len(),
            1,
            "eleven observations, one announcement: {first_ten:?}"
        );
        let to_a_minute = play_through(&mut scrobbler, THREE_MINUTES, 60);
        assert_eq!(
            to_a_minute
                .iter()
                .filter(|report| matches!(report, Report::NowPlaying { .. }))
                .count(),
            2,
            "one at thirty seconds and one at sixty: {to_a_minute:?}"
        );
    }

    /// Rule: a new song starts its own count, and the previous song's
    /// progress does not carry over to it.
    #[test]
    fn the_next_song_starts_from_nothing() {
        let mut scrobbler = Scrobbler::new();
        play_through(&mut scrobbler, THREE_MINUTES, 179);
        assert!(scrobbler.submitted());

        let reports = scrobbler.observe(Some("s2"), Duration::ZERO, THREE_MINUTES, true, 180_000);
        assert_eq!(
            reports,
            vec![Report::NowPlaying {
                id: "s2".to_string()
            }]
        );
        assert!(!scrobbler.submitted());
    }

    /// Rule: the submission carries when the play *started*, not when it
    /// passed the halfway mark, so history reads in the order it happened.
    #[test]
    fn the_submission_carries_the_time_the_play_started() {
        let mut scrobbler = Scrobbler::new();
        let base = 1_788_299_852_000_u64;
        scrobbler.observe(Some(SONG), Duration::ZERO, THREE_MINUTES, true, base);
        let mut submitted = None;
        for second in 1..=95 {
            for report in scrobbler.observe(
                Some(SONG),
                Duration::from_secs(second),
                THREE_MINUTES,
                true,
                base + second * 1000,
            ) {
                if let Report::Played { started_at_ms, .. } = report {
                    submitted = Some(started_at_ms);
                }
            }
        }
        assert_eq!(submitted, Some(base));
    }

    /// Rule: nothing playing reports nothing, and forgets what was.
    #[test]
    fn silence_reports_nothing() {
        let mut scrobbler = Scrobbler::new();
        play_through(&mut scrobbler, THREE_MINUTES, 20);
        assert!(
            scrobbler
                .observe(None, Duration::ZERO, Duration::ZERO, false, 21_000)
                .is_empty()
        );
        assert!(!scrobbler.submitted());
        scrobbler.stopped();
        assert!(!scrobbler.submitted());
    }
}

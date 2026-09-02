//! The contract the interface plays through.
//!
//! These are the types `src/backend.rs` and `src/app.rs` speak to the
//! engine with. They are carried over from the librespot engine in
//! `src/player.rs` deliberately — `migration/02-audio-engine.md` keeps the
//! contract so that the wire-up in Phase 4 is an import change rather than a
//! rewrite of the interface.
//!
//! Two fields did not come across, because nothing behind them exists any
//! more: `LocalState::active_client` (which Connect client is driving this
//! device). `EngineEvent` lost
//! `SessionEnded` for the same reason: an HTTP client has no session to
//! drop.
//!
//! Two things were added, because the queue came home from Spirc with the
//! rest of Connect: `PlayerCommand::PlayQueued`, which plays a row of the
//! queue, and `EngineEvent::Queue`, which is how the interface learns what
//! the queue holds now that no web API can be asked.

use std::time::Instant;

use super::queue::QueueSnapshot;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Playback {
    #[default]
    Stopped,
    Loading,
    Playing,
    Paused,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RepeatMode {
    #[default]
    Off,
    Context,
    Track,
}

impl RepeatMode {
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Context,
            Self::Context => Self::Track,
            Self::Track => Self::Off,
        }
    }

    pub fn api_name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Context => "context",
            Self::Track => "track",
        }
    }

    pub fn from_api(name: &str) -> Self {
        match name {
            "context" => Self::Context,
            "track" => Self::Track,
            _ => Self::Off,
        }
    }
}

/// What is playing, in the vocabulary the player bar draws.
///
/// `art_url` and `art_small_url` are cover-art *requests*
/// (`sonic:art:<size>:<id>`), not URLs: `src/images.rs` turns them into a
/// `getCoverArt` call with the current credential when it fetches. See D5
/// and the artwork note in `migration/PROGRESS.md`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalTrack {
    pub uri: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub art_url: Option<String>,
    pub art_small_url: Option<String>,
    pub duration_ms: u32,
    /// Whether the server said this song is starred, as it said it with the
    /// song. `None` for a row the queue knows only by URI, which has not
    /// been read from the server yet. See `api::models::Starred`.
    pub starred: Option<bool>,
}

impl LocalTrack {
    pub fn artist_names(&self) -> String {
        self.artists.join(", ")
    }
}

/// The snapshot pushed to the interface whenever something changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalState {
    pub playback: Playback,
    pub track: Option<LocalTrack>,
    pub position_ms: u32,
    /// When `position_ms` was observed; `None` while not advancing.
    pub position_at: Option<Instant>,
    pub volume: u16,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    /// Whether the engine has credentials for a server to stream from.
    pub connected: bool,
    pub username: String,
    pub error: Option<String>,
    pub seek_sequence: u64,
}

/// What playback was doing when its engine was replaced, so the next one
/// can pick it up. Changing the output device rebuilds the engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interrupted {
    pub uri: String,
    pub position_ms: u32,
    /// Playing or loading, as opposed to paused.
    pub playing: bool,
}

impl LocalState {
    /// The track and position to come back to, if something was on.
    pub fn interrupted(&self) -> Option<Interrupted> {
        let track = self.track.as_ref()?;
        if self.playback == Playback::Stopped {
            return None;
        }
        Some(Interrupted {
            uri: track.uri.clone(),
            position_ms: self.position_now(),
            playing: matches!(self.playback, Playback::Playing | Playback::Loading),
        })
    }

    /// The position now, interpolated from the last report while playing.
    ///
    /// The engine reports a corrected position about twice a second; this is
    /// what lets the progress bar move smoothly between two of them.
    pub fn position_now(&self) -> u32 {
        match (self.playback, self.position_at) {
            (Playback::Playing, Some(at)) => {
                let elapsed = at.elapsed().as_millis() as u32;
                let limit = self
                    .track
                    .as_ref()
                    .map_or(u32::MAX, |track| track.duration_ms.max(self.position_ms));
                self.position_ms.saturating_add(elapsed).min(limit)
            }
            _ => self.position_ms,
        }
    }

    pub fn is_active(&self) -> bool {
        self.track.is_some() && self.playback != Playback::Stopped
    }
}

/// The queue and the track, as a load the next engine can be given: what
/// changing the output device or the normalisation switch has to carry
/// across, since the queue lives in the engine (rule 9).
///
/// `None` when there is nothing to carry. The context is named rather than
/// listed wherever it has a name, so the new engine reloads an album in one
/// request instead of asking about every track in it; a bare list of songs
/// has no name and travels as its rows.
pub fn carry_over(state: &LocalState, queue: &QueueSnapshot) -> Option<LoadSpec> {
    let interrupted = state.interrupted();
    let current = queue.current.as_ref().map(|row| row.uri.clone());
    let queued: Vec<String> = queue.queued.iter().map(|row| row.uri.clone()).collect();
    if current.is_none() && queued.is_empty() && queue.upcoming.is_empty() {
        return None;
    }
    let position_ms = interrupted.as_ref().map_or(0, |resume| resume.position_ms);
    let play = interrupted.as_ref().is_some_and(|resume| resume.playing);
    let mut spec = LoadSpec {
        position_ms,
        play,
        shuffle: Some(state.shuffle),
        queued,
        ..LoadSpec::default()
    };
    match &queue.context_uri {
        Some(context) => {
            spec.context_uri = Some(context.clone());
            // The album keeps its place under a queued song, so the offset
            // is where the album is and `current` is what is heard.
            spec.offset_uri = queue.context_at.clone().or_else(|| current.clone());
            if queue.context_at.is_some() && queue.context_at != current {
                spec.current = current;
            }
        }
        None => {
            // No album to name: the rows are the list, and the song playing
            // is its first row.
            spec.uris = current
                .into_iter()
                .chain(queue.upcoming.iter().map(|row| row.uri.clone()))
                .collect();
            spec.offset_index = Some(0);
        }
    }
    Some(spec)
}

/// What to play, and where to start.
///
/// `context_uri` is an album or a playlist to play in order; `uris` is a
/// list of tracks to play as it stands. The old service's `autoplay` — the station
/// it invented once a context ran out — has no equivalent and is gone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadSpec {
    pub context_uri: Option<String>,
    pub uris: Vec<String>,
    pub offset_uri: Option<String>,
    pub offset_index: Option<u32>,
    pub position_ms: u32,
    pub play: bool,
    pub shuffle: Option<bool>,
    /// Songs to put back into "Playing next", oldest first — rule 9 of
    /// `docs/_reference/queue.md`. Only a restore sets this: the app
    /// resuming the session it last closed, or an engine handing its queue
    /// to the one replacing it. Every other load leaves the queued songs
    /// where they are, which is rule 6.
    pub queued: Vec<String>,
    /// The song playing, when it was one of the queued ones rather than
    /// the context's own row (rule 3). `offset_uri` then says where the
    /// album is, and this says what is heard over the top of it.
    pub current: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlayerCommand {
    Toggle,
    Next,
    Previous,
    /// Remove manually queued tracks and keep context tracks.
    ClearQueue,
    /// Queue a track after the ones already queued.
    AddToQueue(String),
    /// Play the row at this position in the queue, counting the rows as
    /// `QueueSnapshot` draws them: "Playing next" first, then "Next up".
    /// The rows above it are skipped, which is rule 5 of
    /// `docs/_reference/queue.md`. The old remote path answered this with a series of
    /// Next commands over Connect; the engine does it in one step.
    PlayQueued(usize),
    Seek(u32),
    /// The volume to keep.
    Volume(u16),
    /// The slider mid-drag. Connect made this cheaper than `Volume`, which
    /// cost a round trip for every value dragged through. Local
    /// playback makes them the same thing; the variant stays because the
    /// Winamp slider and the mixer window both send it.
    VolumePreview(u16),
    Shuffle(bool),
    Repeat(RepeatMode),
    Load(LoadSpec),
    /// Connect's "take over this device". Nothing to take over now.
    Activate,
}

#[derive(Clone, Debug)]
pub enum EngineEvent {
    State(LocalState),
    /// What is playing next. Pushed only when it changes, because it is a
    /// list rather than a line and most state changes leave it alone.
    Queue(QueueSnapshot),
}

pub type Notify = std::sync::Arc<dyn Fn(EngineEvent) + Send + Sync>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn position_interpolates_only_while_playing() {
        let mut state = LocalState {
            playback: Playback::Paused,
            position_ms: 5_000,
            position_at: Some(Instant::now() - Duration::from_secs(2)),
            ..LocalState::default()
        };
        assert_eq!(state.position_now(), 5_000);
        state.playback = Playback::Playing;
        assert!(state.position_now() >= 7_000);
    }

    /// Interpolation never runs past the end of the track: a state that was
    /// pushed just before the last packet drained must not show a position
    /// beyond the duration while the next track loads.
    #[test]
    fn interpolation_stops_at_the_end_of_the_track() {
        let state = LocalState {
            playback: Playback::Playing,
            track: Some(LocalTrack {
                duration_ms: 6_000,
                ..LocalTrack::default()
            }),
            position_ms: 5_900,
            position_at: Some(Instant::now() - Duration::from_secs(10)),
            ..LocalState::default()
        };
        assert_eq!(state.position_now(), 6_000);
    }

    fn row(uri: &str) -> crate::engine::QueueRow {
        crate::engine::QueueRow {
            uri: uri.into(),
            track: None,
        }
    }

    /// A state and a queue as the engine holds them while an album plays.
    fn playing_album() -> (LocalState, QueueSnapshot) {
        let state = LocalState {
            playback: Playback::Playing,
            track: Some(LocalTrack {
                uri: "sonic:track:b".into(),
                duration_ms: 300_000,
                ..LocalTrack::default()
            }),
            position_ms: 30_000,
            ..LocalState::default()
        };
        let queue = QueueSnapshot {
            current: Some(row("sonic:track:b")),
            queued: vec![row("sonic:track:q1")],
            upcoming: vec![row("sonic:track:c")],
            context_uri: Some("sonic:album:x".into()),
            context_at: Some("sonic:track:b".into()),
        };
        (state, queue)
    }

    /// Rule 9 across an engine replacement: the album travels by name and
    /// the songs queued by hand travel with it, at the position the old
    /// engine had reached.
    #[test]
    fn a_replaced_engine_is_handed_the_whole_queue() {
        let (state, queue) = playing_album();
        let spec = carry_over(&state, &queue).expect("something was playing");
        assert_eq!(spec.context_uri.as_deref(), Some("sonic:album:x"));
        assert_eq!(spec.offset_uri.as_deref(), Some("sonic:track:b"));
        assert_eq!(spec.queued, vec!["sonic:track:q1"]);
        assert_eq!(spec.current, None);
        assert!(spec.uris.is_empty(), "an album is named, not listed");
        assert!(spec.play);
        assert!(spec.position_ms >= 30_000);
    }

    /// A queued song playing over the album keeps both: the song is picked
    /// up again and the album stays on the row it interrupted (rule 3).
    #[test]
    fn a_queued_song_is_carried_over_the_album_it_interrupted() {
        let (mut state, mut queue) = playing_album();
        state.track = Some(LocalTrack {
            uri: "sonic:track:q0".into(),
            ..LocalTrack::default()
        });
        queue.current = Some(row("sonic:track:q0"));
        let spec = carry_over(&state, &queue).expect("something was playing");
        assert_eq!(spec.offset_uri.as_deref(), Some("sonic:track:b"));
        assert_eq!(spec.current.as_deref(), Some("sonic:track:q0"));
        assert_eq!(spec.queued, vec!["sonic:track:q1"]);
    }

    /// A paused engine is replaced paused, and a list of songs with no
    /// album to name travels as its rows.
    #[test]
    fn a_bare_list_is_carried_as_its_rows() {
        let (mut state, mut queue) = playing_album();
        state.playback = Playback::Paused;
        queue.context_uri = None;
        queue.context_at = None;
        let spec = carry_over(&state, &queue).expect("something was paused");
        assert!(!spec.play, "a paused player comes back paused");
        assert_eq!(spec.context_uri, None);
        assert_eq!(spec.uris, vec!["sonic:track:b", "sonic:track:c"]);
        assert_eq!(spec.offset_index, Some(0));
        assert_eq!(spec.queued, vec!["sonic:track:q1"]);
    }

    /// Nothing playing and nothing queued: there is nothing to carry, and
    /// the new engine is left alone rather than told to play something.
    #[test]
    fn an_empty_queue_carries_nothing() {
        assert!(carry_over(&LocalState::default(), &QueueSnapshot::default()).is_none());
    }

    #[test]
    fn repeat_cycles_and_maps() {
        assert_eq!(RepeatMode::Off.next(), RepeatMode::Context);
        assert_eq!(RepeatMode::Track.next(), RepeatMode::Off);
        assert_eq!(RepeatMode::from_api("track"), RepeatMode::Track);
        assert_eq!(RepeatMode::Context.api_name(), "context");
    }

    /// A track that was playing or paused is remembered with its position;
    /// nothing is, once playback has stopped.
    #[test]
    fn an_interrupted_track_is_remembered_with_its_position() {
        let mut state = LocalState {
            track: Some(LocalTrack {
                uri: "sonic:track:x".into(),
                duration_ms: 200_000,
                ..LocalTrack::default()
            }),
            playback: Playback::Playing,
            position_ms: 10_000,
            position_at: Some(Instant::now()),
            ..LocalState::default()
        };
        let resume = state.interrupted().expect("playing");
        assert_eq!(resume.uri, "sonic:track:x");
        assert!(resume.playing);
        assert!(resume.position_ms >= 10_000);
        state.playback = Playback::Paused;
        assert!(!state.interrupted().expect("paused").playing);
        state.playback = Playback::Stopped;
        assert!(state.interrupted().is_none());
        state.playback = Playback::Playing;
        state.track = None;
        assert!(state.interrupted().is_none());
    }
}

//! The queue: what is playing, what you queued, and what the album has
//! left.
//!
//! Spirc owned this on the other side of a network. It is ours now, and
//! `docs/_reference/queue.md` is what it has to do — nine rules, written
//! for the person using the app. Rules 1 to 7 are held up by the tests in
//! this file; 8 and 9 are the app's, in `src/app.rs`. (There was a tenth,
//! about ignoring stale remote answers, and P4.3 retired it: there
//! is no second copy of the queue to be stale.) The numbered comments
//! through this file point back at those rules.
//!
//! The shape is the one the interface draws:
//!
//! ```text
//!   current           the song playing; it is in neither list (rule 3)
//!   queued    ─┐      "Playing next" — what you asked for, oldest first
//!   upcoming  ─┴──►   "Next up" — what the album or playlist has left
//! ```
//!
//! Nothing in here touches audio or the network, so all of it is tested
//! without either. What is playing is *where*, not a copy: a context track
//! is the entry at `at` in `order`, so the metadata filled in by
//! `Queue::learn` is filled in once and seen everywhere.

use std::collections::VecDeque;
use std::time::Duration;

use rand::seq::SliceRandom;

use crate::api::models::pick_image;
use crate::api::subsonic::{Child, convert};

use super::state::{LocalTrack, RepeatMode};

/// Previous goes back a track only near the start of one; after this it
/// restarts the track being played, as every music player has since the CD.
const PREVIOUS_RESTARTS_AFTER: Duration = Duration::from_secs(3);

/// One track in the queue: the id it plays by, and the server's record of
/// it once anything has needed it.
///
/// An album or a playlist arrives with every song in it, so those entries
/// are born complete. A track queued by hand, or a bare list of uris to
/// play, arrives as an id and is filled in later — by the engine when it
/// opens the track, or by [`Queue::learn`] while there is nothing else to
/// do.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Entry {
    pub id: String,
    pub song: Option<Child>,
}

impl Entry {
    pub(super) fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            song: None,
        }
    }

    pub(super) fn known(song: Child) -> Self {
        Self {
            id: song.id.clone(),
            song: Some(song),
        }
    }

    fn row(&self) -> QueueRow {
        QueueRow {
            uri: convert::track_uri(&self.id),
            track: self.song.as_ref().map(local_track),
        }
    }
}

/// Which of the two lists the song being played came from. A queued song
/// leaves its list when it starts (rule 3) but the difference still
/// matters: the context stays where it was underneath it, and Previous
/// goes back to it.
#[derive(Clone, Debug, PartialEq)]
enum Playing {
    /// Boxed because a described song is a kilobyte of metadata and this
    /// is the rare half of the enum.
    Queued(Box<Entry>),
    /// The context track at `at`.
    Context,
}

/// One row of the queue as the interface draws it. `track` is missing only
/// until the server has been asked about that id.
#[derive(Clone, Debug, PartialEq)]
pub struct QueueRow {
    pub uri: String,
    pub track: Option<LocalTrack>,
}

/// The queue, for drawing. Rule 1: this is the play order — `queued` first,
/// then `upcoming`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QueueSnapshot {
    pub current: Option<QueueRow>,
    /// "Playing next": the songs queued by hand, in the order they play.
    pub queued: Vec<QueueRow>,
    /// "Next up": what is left of the album or playlist that is playing.
    pub upcoming: Vec<QueueRow>,
    /// The album, playlist or artist being played, if it came from one.
    pub context_uri: Option<String>,
    /// The context row the album or playlist has got to, which is `current`
    /// unless a song you queued is playing over the top of it (rule 3).
    /// `None` before the context has started. Rule 9's restore needs both:
    /// the song to play, and the place the album keeps underneath it.
    pub context_at: Option<String>,
}

impl QueueSnapshot {
    /// The rows below the one playing, in play order — which is how
    /// `Queue::skip_to` counts them.
    pub fn rows(&self) -> impl Iterator<Item = &QueueRow> {
        self.queued.iter().chain(self.upcoming.iter())
    }

    pub fn is_empty(&self) -> bool {
        self.queued.is_empty() && self.upcoming.is_empty()
    }
}

/// What Previous decided to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Rewound {
    /// A different track is current now; open it.
    Moved,
    /// Play the same track from the beginning.
    Restart,
}

#[derive(Debug, Default)]
pub(super) struct Queue {
    /// "Playing next", oldest first. Rule 2: the same song queued twice is
    /// two rows and plays twice.
    queued: VecDeque<Entry>,
    /// The album or playlist, in the order the server gave it.
    context: Vec<Entry>,
    /// Positions into `context`, in play order. Shuffle rewrites this and
    /// nothing else, so turning it off restores the album.
    order: Vec<usize>,
    /// How far along `order` the context has got. `None` before anything
    /// in it has played, which is also where the end of the list winds back
    /// to.
    at: Option<usize>,
    playing: Option<Playing>,
    context_uri: Option<String>,
    shuffle: bool,
}

impl Queue {
    /// The song playing, or the one that would start if play were pressed.
    pub(super) fn current(&self) -> Option<&Entry> {
        match self.playing.as_ref()? {
            Playing::Queued(entry) => Some(entry),
            Playing::Context => self.context_row(),
        }
    }

    /// The context row the album has got to, playing or not.
    fn context_row(&self) -> Option<&Entry> {
        self.context.get(*self.order.get(self.at?)?)
    }

    fn current_mut(&mut self) -> Option<&mut Entry> {
        match self.playing.as_mut()? {
            Playing::Queued(entry) => Some(entry),
            Playing::Context => {
                let index = *self.order.get(self.at?)?;
                self.context.get_mut(index)
            }
        }
    }

    /// Nothing to play at all — no context, nothing queued, nothing open.
    pub(super) fn is_empty(&self) -> bool {
        self.context.is_empty() && self.queued.is_empty() && self.playing.is_none()
    }

    /// Play this album, playlist or list of songs, starting at `start`.
    ///
    /// Rule 6: the songs queued by hand are kept and still play first. The
    /// context underneath them is what changes.
    pub(super) fn load(&mut self, context_uri: Option<String>, entries: Vec<Entry>, start: usize) {
        if entries.is_empty() {
            return;
        }
        let start = start.min(entries.len() - 1);
        self.context_uri = context_uri;
        self.context = entries;
        self.order = (0..self.context.len()).collect();
        if self.shuffle {
            self.reorder_around(start);
        } else {
            self.at = Some(start);
        }
        self.playing = Some(Playing::Context);
    }

    /// Rule 2: after the songs queued earlier, before the album's.
    pub(super) fn add(&mut self, entry: Entry) {
        self.queued.push_back(entry);
    }

    /// Rule 9: put a queued song back as the one playing, leaving the
    /// album where it was.
    ///
    /// This is the state a restore cannot reach through `load` and `add`:
    /// a song you queued plays *over* the context, which stays at the
    /// track it interrupted (rule 3), so a resumed session has to be told
    /// both. Nothing else calls it — a queued song reaches this state by
    /// being advanced onto, which is where it also leaves the queue.
    pub(super) fn play_queued_now(&mut self, entry: Entry) {
        self.playing = Some(Playing::Queued(Box::new(entry)));
    }

    /// Rule 7: Clear empties "Playing next" and leaves the album alone.
    pub(super) fn clear_queued(&mut self) {
        self.queued.clear();
    }

    /// The next song: yours first, then the album's. `false` means the list
    /// has run out — repeat one is the caller's business, because pressing
    /// Next with it on still moves on.
    ///
    /// Rules 3 and 4: a song that starts is no longer a row in the queue,
    /// and the row goes the moment Next is pressed.
    pub(super) fn advance(&mut self, repeat: RepeatMode) -> bool {
        if let Some(entry) = self.queued.pop_front() {
            self.playing = Some(Playing::Queued(Box::new(entry)));
            return true;
        }
        match self.next_position(repeat) {
            Some(position) => {
                self.at = Some(position);
                self.playing = Some(Playing::Context);
                true
            }
            None => false,
        }
    }

    /// What [`Queue::advance`] would land on, without moving.
    ///
    /// The engine opens the next track while this one is still playing
    /// (P3.4), and the queue must not move until the sound does — so the
    /// question "what is next" and the act of going there are two different
    /// calls. Repeat one is the caller's business here too: it does not
    /// change what is *next*, only what happens at the end of a track.
    pub(super) fn peek_next(&self, repeat: RepeatMode) -> Option<&Entry> {
        if let Some(entry) = self.queued.front() {
            return Some(entry);
        }
        let position = self.next_position(repeat)?;
        self.context.get(*self.order.get(position)?)
    }

    fn next_position(&self, repeat: RepeatMode) -> Option<usize> {
        if self.order.is_empty() {
            return None;
        }
        match self.at {
            None => Some(0),
            Some(at) if at + 1 < self.order.len() => Some(at + 1),
            Some(_) => (repeat == RepeatMode::Context).then_some(0),
        }
    }

    /// Previous, from `position` into the track being played.
    ///
    /// A queued song goes back to the album track it interrupted, which is
    /// where it was inserted; the queued song itself is gone, because it
    /// left the queue when it started.
    pub(super) fn previous(&mut self, repeat: RepeatMode, position: Duration) -> Rewound {
        if self.playing.is_none() || position >= PREVIOUS_RESTARTS_AFTER {
            return Rewound::Restart;
        }
        if matches!(self.playing, Some(Playing::Queued(_))) {
            return match self.at {
                Some(_) => {
                    self.playing = Some(Playing::Context);
                    Rewound::Moved
                }
                None => Rewound::Restart,
            };
        }
        let Some(at) = self.at else {
            return Rewound::Restart;
        };
        let previous = if at > 0 {
            at - 1
        } else if repeat == RepeatMode::Context && !self.order.is_empty() {
            self.order.len() - 1
        } else {
            return Rewound::Restart;
        };
        self.at = Some(previous);
        self.playing = Some(Playing::Context);
        Rewound::Moved
    }

    /// Rule 5: playing a row skips to it. The rows above go, as if Next had
    /// been pressed down to it; the rows below stay and the album keeps
    /// going afterwards.
    ///
    /// `row` counts the rows as [`QueueSnapshot`] draws them: "Playing
    /// next" first, then "Next up".
    pub(super) fn skip_to(&mut self, row: usize) -> bool {
        if row < self.queued.len() {
            self.queued.drain(..row);
            let entry = self.queued.pop_front().expect("the row is in the queue");
            self.playing = Some(Playing::Queued(Box::new(entry)));
            return true;
        }
        let position = match self.at {
            Some(at) => at + 1,
            None => 0,
        } + (row - self.queued.len());
        if position >= self.order.len() {
            return false;
        }
        // Every queued song is above this row, so every one of them is
        // skipped past.
        self.queued.clear();
        self.at = Some(position);
        self.playing = Some(Playing::Context);
        true
    }

    /// The end of the list, with nothing repeating. The context stays,
    /// wound back to its start: pressing play after an album has run out
    /// plays the album, which is what the button appears to promise.
    pub(super) fn rewind(&mut self) {
        self.at = None;
        self.playing = None;
    }

    /// Shuffle rewrites the play order and keeps the album's own order
    /// underneath it, so turning it off puts the album back — at the track
    /// that is playing, not at the top.
    pub(super) fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
        if self.context.is_empty() {
            return;
        }
        let playing = self.at.and_then(|at| self.order.get(at).copied());
        match (on, playing) {
            (true, Some(keep)) => self.reorder_around(keep),
            (true, None) => {
                self.order.shuffle(&mut rand::rng());
                self.at = None;
            }
            (false, playing) => {
                self.order = (0..self.context.len()).collect();
                // In the album's own order a track's position *is* its
                // index.
                self.at = playing;
            }
        }
    }

    /// A shuffled order with `keep` — an index into `context` — at the
    /// front, so that the track playing is not interrupted by the shuffle
    /// and does not come round again straight after it.
    fn reorder_around(&mut self, keep: usize) {
        let mut rest: Vec<usize> = (0..self.context.len()).filter(|at| *at != keep).collect();
        rest.shuffle(&mut rand::rng());
        self.order = std::iter::once(keep).chain(rest).collect();
        self.at = Some(0);
    }

    /// The server's record of the song being played, once it has one.
    pub(super) fn learn_current(&mut self, song: Child) {
        if let Some(entry) = self.current_mut() {
            entry.song = Some(song.clone());
        }
        self.learn(&song);
    }

    /// Fill in a song wherever it appears, so that a track queued twice is
    /// asked about once.
    pub(super) fn learn(&mut self, song: &Child) {
        let fill = |entry: &mut Entry| {
            if entry.id == song.id && entry.song.is_none() {
                entry.song = Some(song.clone());
            }
        };
        self.context.iter_mut().for_each(fill);
        self.queued.iter_mut().for_each(fill);
        if let Some(Playing::Queued(entry)) = self.playing.as_mut() {
            fill(entry);
        }
    }

    /// The next song nobody has asked the server about, among the rows the
    /// interface can see: what is playing, everything queued, and the first
    /// `depth` rows of the album. Filling these in is what lets the queue
    /// be drawn as songs rather than as ids.
    pub(super) fn unknown(&self, depth: usize) -> Option<String> {
        self.current()
            .into_iter()
            .chain(self.queued.iter())
            .chain(self.upcoming().take(depth))
            .find(|entry| entry.song.is_none())
            .map(|entry| entry.id.clone())
    }

    /// What the context has left after the track playing, in play order.
    fn upcoming(&self) -> impl Iterator<Item = &Entry> {
        let from = match self.at {
            Some(at) => at + 1,
            None => 0,
        };
        self.order
            .iter()
            .skip(from)
            .filter_map(|index| self.context.get(*index))
    }

    pub(super) fn snapshot(&self) -> QueueSnapshot {
        QueueSnapshot {
            current: self.current().map(Entry::row),
            queued: self.queued.iter().map(Entry::row).collect(),
            upcoming: self.upcoming().map(Entry::row).collect(),
            context_uri: self.context_uri.clone(),
            context_at: self
                .context_row()
                .map(|entry| convert::track_uri(&entry.id)),
        }
    }
}

/// One song, as the player bar reads it.
///
/// The artwork is a request rather than a URL — `sonic:art:<size>:<id>` —
/// which `src/images.rs` turns into a `getCoverArt` call at fetch time.
pub(super) fn local_track(song: &Child) -> LocalTrack {
    let track = convert::track(song);
    let images = track
        .album
        .as_ref()
        .map(|album| album.images.as_slice())
        .unwrap_or_default();
    LocalTrack {
        uri: track.uri,
        title: track.name,
        artists: track
            .artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect(),
        album: track
            .album
            .as_ref()
            .map(|album| album.name.clone())
            .unwrap_or_default(),
        art_url: pick_image(images, 640).map(str::to_string),
        art_small_url: pick_image(images, 64).map(str::to_string),
        duration_ms: track.duration_ms,
        starred: track.starred,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(ids: &[&str]) -> Vec<Entry> {
        ids.iter().map(|id| Entry::new(*id)).collect()
    }

    /// An album loaded and playing from its first track.
    fn album(ids: &[&str]) -> Queue {
        let mut queue = Queue::default();
        queue.load(Some("sonic:album:a1".into()), entries(ids), 0);
        queue
    }

    fn playing(queue: &Queue) -> Option<&str> {
        queue.current().map(|entry| entry.id.as_str())
    }

    /// The rows below the one playing, in the order they will play.
    fn rows(queue: &Queue) -> Vec<String> {
        queue
            .snapshot()
            .rows()
            .map(|row| row.uri.trim_start_matches("sonic:track:").to_string())
            .collect()
    }

    fn next(queue: &mut Queue) -> Option<&str> {
        queue.advance(RepeatMode::Off);
        playing(queue)
    }

    /// A queue in a named state, built to order so that peeking at it and
    /// advancing it can be compared on two identical copies.
    type State = (&'static str, fn() -> Queue);

    fn peeked(queue: &Queue, repeat: RepeatMode) -> Option<&str> {
        queue.peek_next(repeat).map(|entry| entry.id.as_str())
    }

    /// P3.4: the engine opens the next track while this one plays, so
    /// "what is next" has to be answerable without going there — and it
    /// has to be the same answer that going there gives, in every state
    /// the queue can be in. A prefetch that disagreed with `advance`
    /// would play the wrong song at the join.
    #[test]
    fn what_plays_next_is_what_advance_lands_on() {
        let states: Vec<State> = vec![
            ("an album", || album(&["a", "b", "c"])),
            ("a queued song first", || {
                let mut queue = album(&["a", "b"]);
                queue.add(Entry::new("q1"));
                queue
            }),
            ("the last track of an album", || {
                let mut queue = album(&["a", "b"]);
                queue.advance(RepeatMode::Off);
                queue
            }),
            ("nothing playing yet", Queue::default),
            ("an album wound back to its start", || {
                let mut queue = album(&["a", "b"]);
                queue.rewind();
                queue
            }),
        ];
        for repeat in [RepeatMode::Off, RepeatMode::Context, RepeatMode::Track] {
            for (name, build) in &states {
                let queue = build();
                let peeked = peeked(&queue, repeat).map(str::to_string);
                let mut moved = build();
                let advanced = moved
                    .advance(repeat)
                    .then(|| playing(&moved).map(str::to_string))
                    .flatten();
                assert_eq!(
                    peeked, advanced,
                    "{name} with repeat {repeat:?}: peeked {peeked:?}, advance landed on {advanced:?}"
                );
            }
        }
    }

    /// Rule 1: the snapshot is the play order, yours before the album's.
    #[test]
    fn the_list_shows_the_play_order() {
        let mut queue = album(&["a", "b", "c"]);
        queue.add(Entry::new("q1"));
        queue.add(Entry::new("q2"));
        assert_eq!(playing(&queue), Some("a"));
        assert_eq!(rows(&queue), ["q1", "q2", "b", "c"]);
        let snapshot = queue.snapshot();
        assert_eq!(snapshot.queued.len(), 2);
        assert_eq!(snapshot.upcoming.len(), 2);
        assert_eq!(snapshot.context_uri.as_deref(), Some("sonic:album:a1"));
        assert_eq!(
            snapshot.current.map(|row| row.uri),
            Some("sonic:track:a".to_string())
        );
    }

    /// Rule 2: Play next goes after the songs queued earlier and before the
    /// album's, and the same song queued twice plays twice.
    #[test]
    fn play_next_queues_after_the_songs_queued_earlier() {
        let mut queue = album(&["a", "b"]);
        queue.add(Entry::new("q1"));
        queue.add(Entry::new("q1"));
        queue.add(Entry::new("q2"));
        assert_eq!(rows(&queue), ["q1", "q1", "q2", "b"]);
        assert_eq!(next(&mut queue), Some("q1"));
        assert_eq!(next(&mut queue), Some("q1"));
        assert_eq!(next(&mut queue), Some("q2"));
        assert_eq!(next(&mut queue), Some("b"));
    }

    /// Rules 3 and 4: a song that starts is not also a row waiting to
    /// play, however it started.
    #[test]
    fn a_song_that_starts_leaves_the_queue() {
        let mut queue = album(&["a", "b"]);
        queue.add(Entry::new("q1"));
        assert_eq!(next(&mut queue), Some("q1"));
        assert_eq!(rows(&queue), ["b"]);
        assert!(queue.snapshot().queued.is_empty());
        // And the album underneath is where it was, so it carries on from
        // the track the queued song interrupted.
        assert_eq!(next(&mut queue), Some("b"));
    }

    /// Rule 5: playing a row skips to it — the rows above go, the rows
    /// below stay, and the album keeps going afterwards.
    #[test]
    fn playing_a_row_skips_to_it() {
        let mut queue = album(&["a", "b", "c", "d"]);
        queue.add(Entry::new("q1"));
        queue.add(Entry::new("q2"));
        assert_eq!(rows(&queue), ["q1", "q2", "b", "c", "d"]);
        // The second queued song: the first is skipped, the album is not
        // touched.
        assert!(queue.skip_to(1));
        assert_eq!(playing(&queue), Some("q2"));
        assert_eq!(rows(&queue), ["b", "c", "d"]);
        // A row in the album: everything above it, queued or not, goes.
        queue.add(Entry::new("q3"));
        assert!(queue.skip_to(2));
        assert_eq!(playing(&queue), Some("c"));
        assert_eq!(rows(&queue), ["d"]);
        assert_eq!(next(&mut queue), Some("d"));
        // Past the end of the list is not a row, and changes nothing.
        assert!(!queue.skip_to(4));
        assert_eq!(playing(&queue), Some("d"));
    }

    /// Rule 6: a new album keeps the songs you queued, and they still play
    /// first.
    #[test]
    fn a_new_context_keeps_the_queued_songs() {
        let mut queue = album(&["a", "b"]);
        queue.add(Entry::new("q1"));
        queue.load(Some("sonic:playlist:p1".into()), entries(&["x", "y"]), 1);
        assert_eq!(playing(&queue), Some("y"));
        assert_eq!(rows(&queue), ["q1"]);
        assert_eq!(
            queue.snapshot().context_uri.as_deref(),
            Some("sonic:playlist:p1")
        );
        assert_eq!(next(&mut queue), Some("q1"));
        // The old album is gone; the new one has nothing after "y".
        assert!(!queue.advance(RepeatMode::Off));
    }

    /// Rule 7: Clear empties your part of the queue and leaves the album's.
    #[test]
    fn clear_only_removes_the_queued_songs() {
        let mut queue = album(&["a", "b", "c"]);
        queue.add(Entry::new("q1"));
        queue.add(Entry::new("q2"));
        queue.clear_queued();
        assert_eq!(rows(&queue), ["b", "c"]);
        assert_eq!(playing(&queue), Some("a"));
        assert_eq!(next(&mut queue), Some("b"));
    }

    /// The list runs out unless it repeats, and a finished list winds back
    /// so that play starts it again.
    #[test]
    fn the_list_runs_out_unless_it_repeats() {
        let mut queue = album(&["a", "b"]);
        assert!(queue.advance(RepeatMode::Off));
        assert_eq!(playing(&queue), Some("b"));
        assert!(!queue.advance(RepeatMode::Off));
        // Repeat context comes round to the top instead.
        assert!(queue.advance(RepeatMode::Context));
        assert_eq!(playing(&queue), Some("a"));
        // A finished list is wound back rather than emptied.
        queue.advance(RepeatMode::Off);
        assert!(!queue.advance(RepeatMode::Off));
        queue.rewind();
        assert!(queue.current().is_none());
        assert_eq!(rows(&queue), ["a", "b"]);
        assert!(queue.advance(RepeatMode::Off));
        assert_eq!(playing(&queue), Some("a"));
    }

    /// Previous restarts a track that is under way, steps back to the one
    /// before near its start, and wraps only when the list repeats.
    #[test]
    fn previous_restarts_a_track_that_is_under_way() {
        let early = Duration::from_secs(1);
        let late = Duration::from_secs(30);
        let mut queue = album(&["a", "b", "c"]);
        queue.advance(RepeatMode::Off);
        assert_eq!(queue.previous(RepeatMode::Off, late), Rewound::Restart);
        assert_eq!(playing(&queue), Some("b"));
        assert_eq!(queue.previous(RepeatMode::Off, early), Rewound::Moved);
        assert_eq!(playing(&queue), Some("a"));
        // The first track has nothing before it, unless the list repeats.
        assert_eq!(queue.previous(RepeatMode::Off, early), Rewound::Restart);
        assert_eq!(queue.previous(RepeatMode::Context, early), Rewound::Moved);
        assert_eq!(playing(&queue), Some("c"));
    }

    /// Previous out of a queued song goes back to the album track it
    /// interrupted, because that is the song that was playing before it.
    #[test]
    fn previous_out_of_a_queued_song_returns_to_the_album() {
        let mut queue = album(&["a", "b"]);
        queue.add(Entry::new("q1"));
        queue.advance(RepeatMode::Off);
        assert_eq!(playing(&queue), Some("q1"));
        assert_eq!(
            queue.previous(RepeatMode::Off, Duration::from_secs(1)),
            Rewound::Moved
        );
        assert_eq!(playing(&queue), Some("a"));
    }

    /// Shuffle rewrites the play order without disturbing the track
    /// playing, and turning it off puts the album back where it was.
    #[test]
    fn shuffle_reorders_what_is_left_and_can_be_undone() {
        let ids: Vec<String> = (0..24).map(|n| format!("t{n}")).collect();
        let mut queue = Queue::default();
        queue.load(
            Some("sonic:album:a1".into()),
            ids.iter().map(Entry::new).collect(),
            5,
        );
        assert_eq!(playing(&queue), Some("t5"));
        queue.set_shuffle(true);
        // The track playing is untouched, everything else is still there,
        // and it does not come round again next.
        assert_eq!(playing(&queue), Some("t5"));
        let shuffled = rows(&queue);
        assert_eq!(shuffled.len(), 23);
        assert!(!shuffled.contains(&"t5".to_string()));
        let mut sorted = shuffled.clone();
        sorted.sort();
        let mut expected: Vec<String> = ids.iter().filter(|id| *id != "t5").cloned().collect();
        expected.sort();
        assert_eq!(sorted, expected);
        // 23! orders: the album's own is not one this will hit by chance.
        assert_ne!(shuffled, ids[6..].to_vec());
        queue.set_shuffle(false);
        assert_eq!(playing(&queue), Some("t5"));
        assert_eq!(rows(&queue), ids[6..]);
    }

    /// Loading with shuffle already on shuffles what it loads, rather than
    /// playing the album in order until something toggles the button.
    #[test]
    fn a_shuffled_load_is_shuffled_from_the_start() {
        let ids: Vec<String> = (0..24).map(|n| format!("t{n}")).collect();
        let mut queue = Queue::default();
        queue.set_shuffle(true);
        queue.load(
            Some("sonic:album:a1".into()),
            ids.iter().map(Entry::new).collect(),
            0,
        );
        assert_eq!(playing(&queue), Some("t0"));
        assert_ne!(rows(&queue), ids[1..]);
    }

    /// A song the server has described is described everywhere it appears,
    /// including the copy that is playing.
    #[test]
    fn a_song_is_learned_once_and_seen_everywhere() {
        let mut queue = album(&["a", "b"]);
        queue.add(Entry::new("b"));
        queue.add(Entry::new("z"));
        assert_eq!(queue.unknown(10).as_deref(), Some("a"));
        queue.learn_current(Child {
            id: "a".into(),
            title: "Signal Path".into(),
            ..Child::default()
        });
        assert_eq!(queue.unknown(10).as_deref(), Some("b"));
        queue.learn(&Child {
            id: "b".into(),
            title: "Second Light".into(),
            ..Child::default()
        });
        // Both the queued copy and the album's own row know it now.
        let snapshot = queue.snapshot();
        assert_eq!(
            snapshot.queued[0].track.as_ref().map(|t| t.title.clone()),
            Some("Second Light".into())
        );
        assert_eq!(
            snapshot.upcoming[0].track.as_ref().map(|t| t.title.clone()),
            Some("Second Light".into())
        );
        assert_eq!(
            snapshot.current.and_then(|row| row.track).map(|t| t.title),
            Some("Signal Path".into())
        );
        assert_eq!(queue.unknown(10).as_deref(), Some("z"));
    }

    /// An empty queue answers every command without playing anything.
    #[test]
    fn an_empty_queue_stays_empty() {
        let mut queue = Queue::default();
        assert!(queue.is_empty());
        assert!(!queue.advance(RepeatMode::Context));
        assert_eq!(
            queue.previous(RepeatMode::Context, Duration::ZERO),
            Rewound::Restart
        );
        assert!(!queue.skip_to(0));
        queue.set_shuffle(true);
        queue.load(None, Vec::new(), 0);
        assert!(queue.current().is_none());
        assert!(queue.snapshot().is_empty());
        assert_eq!(queue.unknown(10), None);
    }

    /// A load with no context — a handful of songs chosen to play — is a
    /// context like any other, and starts where it was told to.
    #[test]
    fn a_load_starts_where_it_was_asked_to() {
        let mut queue = Queue::default();
        queue.load(None, entries(&["a", "b", "c"]), 2);
        assert_eq!(playing(&queue), Some("c"));
        assert_eq!(queue.snapshot().context_uri, None);
        // An index past the end starts at the last track rather than at
        // nothing.
        queue.load(None, entries(&["a", "b"]), 9);
        assert_eq!(playing(&queue), Some("b"));
    }

    /// The player bar reads artwork as a deferred request, never a URL with
    /// the credential in it.
    #[test]
    fn a_song_becomes_what_the_player_bar_draws() {
        let song = Child {
            id: "s1".into(),
            title: "Signal Path".into(),
            album: Some("Blue Harvest".into()),
            artist: Some("Ravel Kern".into()),
            duration: Some(211),
            cover_art: Some("al-7".into()),
            ..Child::default()
        };
        let track = local_track(&song);
        assert_eq!(track.uri, "sonic:track:s1");
        assert_eq!(track.title, "Signal Path");
        assert_eq!(track.artist_names(), "Ravel Kern");
        assert_eq!(track.album, "Blue Harvest");
        assert_eq!(track.duration_ms, 211_000);
        assert_eq!(track.art_url.as_deref(), Some("sonic:art:640:al-7"));
        assert_eq!(track.art_small_url.as_deref(), Some("sonic:art:64:al-7"));
    }

    /// Rule 9: the queue a session was closed with can be built again —
    /// the album at the row it was on, the songs queued by hand behind it,
    /// and a queued song playing over the top of the album rather than in
    /// place of it (rule 3).
    #[test]
    fn a_saved_queue_can_be_put_back_as_it_was() {
        let mut queue = album(&["a", "b", "c"]);
        queue.load(Some("sonic:album:a1".into()), entries(&["a", "b", "c"]), 1);
        queue.add(Entry::new("q2"));
        queue.play_queued_now(Entry::new("q1"));
        assert_eq!(playing(&queue), Some("q1"), "the queued song is what plays");
        assert_eq!(
            rows(&queue),
            ["q2", "c"],
            "the album carries on from where it was, not from the queued song"
        );
        let snapshot = queue.snapshot();
        assert_eq!(
            snapshot.context_at.as_deref(),
            Some("sonic:track:b"),
            "the album's own row is published beside the song playing, \
             which is what a restore needs to know"
        );
        assert_eq!(
            snapshot.current.map(|row| row.uri),
            Some("sonic:track:q1".into())
        );
        // And it carries on into the album underneath it (rule 3).
        assert_eq!(next(&mut queue), Some("q2"));
        assert_eq!(next(&mut queue), Some("c"));
    }

    /// The context row is what is playing whenever nothing is queued over
    /// it, and nothing at all before the context has started.
    #[test]
    fn the_published_context_row_follows_the_album() {
        let mut queue = Queue::default();
        assert_eq!(queue.snapshot().context_at, None);
        queue.load(Some("sonic:album:a1".into()), entries(&["a", "b"]), 0);
        assert_eq!(
            queue.snapshot().context_at.as_deref(),
            Some("sonic:track:a")
        );
        queue.advance(RepeatMode::Off);
        assert_eq!(
            queue.snapshot().context_at.as_deref(),
            Some("sonic:track:b")
        );
        queue.rewind();
        assert_eq!(
            queue.snapshot().context_at,
            None,
            "an album wound back to its start is on no row at all"
        );
    }

    /// A song with no cover art draws no cover art, rather than a URL that
    /// answers with an error envelope.
    #[test]
    fn a_song_without_art_asks_for_none() {
        let track = local_track(&Child {
            id: "s2".into(),
            title: "Untitled".into(),
            ..Child::default()
        });
        assert_eq!(track.art_url, None);
        assert_eq!(track.art_small_url, None);
        assert_eq!(track.album, "");
    }
}

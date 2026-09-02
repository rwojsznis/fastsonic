//! The audio thread: one loop that takes commands, decodes, and says where
//! the music is.
//!
//! It is one thread on purpose. Spirc used to own the queue, the context,
//! shuffle and repeat, and it lived on the other side of a network; now all
//! of it is here, in front of the decoder, and a command is answered in the
//! time it takes to decode one packet rather than a round trip to Sweden.
//! Nothing in this loop may block on the device or on the network for
//! longer than that: a wait belongs in the loop's own timeout, so a pause
//! is still a pause while a slow server is thinking.
//!
//! The queue itself is not here: it is [`super::queue`], which is pure
//! state and tested as such against `docs/_reference/queue.md`. This module
//! is what turns it into sound — it asks the queue what plays next, opens
//! it, and tells the interface what happened.
//!
//! Two tracks can be open at once (P3.4). Opening one costs three HTTP
//! requests before a note, so the next one is opened while this one is
//! still playing, and at the end of a track its sound goes into the same
//! sink behind it: nothing is heard at the join. That makes the track being
//! *decoded* and the track being *heard* two different things for the last
//! half second of every track, and the interface is told about the one
//! being heard — [`Worker::follow`] is where the difference is kept
//! honest, and it is why the queue does not move until the speaker does.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::api::subsonic::{Child, SubsonicClient, convert};

use super::cache::Cache;
use super::chain::{Chain, replay_gain};
use super::decode::{Decoder, Stream};
use super::output::{Heard, Output, PREFERRED_RATE, TARGET_QUEUE, Token};
use super::queue::{Entry, Queue, QueueSnapshot, Rewound, local_track};
use super::state::{
    EngineEvent, LoadSpec, LocalState, LocalTrack, Notify, Playback, PlayerCommand, RepeatMode,
};
use super::{EngineConfig, Message};

/// How long the loop sleeps when there is nothing to decode.
const IDLE_TICK: Duration = Duration::from_millis(200);
/// How long it sleeps when the output is full and playing.
const BUSY_TICK: Duration = Duration::from_millis(5);
/// How often a playing position is corrected. The interface interpolates
/// between two of these, so this is about drift, not about smoothness.
const POSITION_REPORT: Duration = Duration::from_millis(500);
/// How far down the queue the engine asks the server to describe songs it
/// only knows the id of. Enough for the queue panel to be songs rather than
/// blanks; not the whole of a thousand-track playlist.
const QUEUE_LOOKAHEAD: usize = 40;
/// How long to leave a failed description alone before asking again.
const DESCRIBE_RETRY: Duration = Duration::from_secs(30);
/// How long before the end of a track the next one is opened. Three HTTP
/// requests fit into this comfortably on a home connection, and a track
/// shorter than it is prefetched as soon as it starts.
const PREFETCH_LEAD: Duration = Duration::from_secs(15);
/// How long to leave a track that would not open ahead of time alone. It
/// will be tried again at the join, where the interface can say so.
const PREFETCH_RETRY: Duration = Duration::from_secs(10);

/// A track that is open: the decoder, and where its sound is in the sink.
struct Open {
    decoder: Decoder,
    /// The tag on everything of this track handed to the device, so that
    /// the clock can say when its sound starts and stops.
    token: Token,
    track: LocalTrack,
    /// The position in the track of the next sample to be fed. It is also
    /// what says how much of the track is left to decode, which is when to
    /// open the next one.
    fed: Duration,
    /// The decoder has no more packets; what is queued is the end of the
    /// track.
    drained: bool,
    /// The track's ReplayGain, or one when it has none or the setting is
    /// off. It belongs to the track rather than to the chain because two
    /// tracks are open at a join and each is played at its own.
    gain: f32,
    /// What the queue does when this track becomes the one being heard.
    /// `None` for a track a command started, whose queue move happened
    /// when the command did.
    seam: Option<Seam>,
}

/// What the queue does at a join.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Seam {
    /// The ordinary one: the queue moves on to the next row.
    Advance,
    /// Repeat one: the same row plays again and the queue stays where it
    /// is.
    Again,
}

/// The track after this one, being opened or open, before it is needed.
enum Prefetch {
    /// Being opened, off this thread. There is only ever one, and which
    /// track it is does not matter: what comes back is checked against
    /// what plays next then rather than now.
    Opening,
    /// Open, waiting for the join.
    Ready(Box<Ready>),
    /// It would not open ahead of time. Left alone for a while rather than
    /// asked for again every tick.
    Failed(String, Instant),
}

/// The next track, open and not yet heard.
pub(super) struct Ready {
    id: String,
    song: Child,
    decoder: Decoder,
    /// What the queue will do when this track starts, decided when it was
    /// asked for and checked again at the join.
    seam: Seam,
}

/// The answer to a prefetch, from the runtime back to the audio thread.
pub(crate) struct Opened {
    song: Child,
    decoder: Decoder,
}

/// What the interface reads and is told: the two snapshots it can take
/// without waiting, and the channel changes are pushed down.
pub(super) struct Shared {
    pub(super) notify: Notify,
    pub(super) state: Arc<Mutex<LocalState>>,
    pub(super) queue: Arc<Mutex<QueueSnapshot>>,
}

pub(super) struct Worker {
    client: Arc<SubsonicClient>,
    runtime: tokio::runtime::Handle,
    http: reqwest::blocking::Client,
    commands: Receiver<Message>,
    /// The other end of `commands`, for the answers to the descriptions
    /// asked for in the background: this thread must not wait on the
    /// network for a row of the queue panel.
    replies: Sender<Message>,
    notify: Notify,
    shared: Arc<Mutex<LocalState>>,
    shared_queue: Arc<Mutex<QueueSnapshot>>,
    state: LocalState,
    output: Output,
    /// The equalizer, the visualiser tap and the limiter, between the
    /// decoder and the device.
    chain: Chain,
    /// Whether tracks play at their ReplayGain. From Settings, and a change
    /// to it replaces the engine the way a change of output device does.
    normalisation: bool,
    /// Where the bytes of a stream are kept so that playing a track again
    /// asks the server for nothing. `None` with the cache turned off.
    cache: Option<Arc<Cache>>,
    /// The tracks whose sound is in the sink, in the order they are heard:
    /// the one playing, and — for the last half second of a track — the one
    /// joined onto the end of it. Never more than two.
    open: VecDeque<Open>,
    /// The next tag to hand out. It only has to be different from the last
    /// one; nothing counts with it.
    tokens: Token,
    queue: Queue,
    /// The track after the one being decoded, opened before it is needed.
    prefetch: Option<Prefetch>,
    /// Whether a description has been asked for and not yet answered.
    describing: bool,
    /// When the last description failed, so a server that is down is not
    /// asked again every tick.
    described_failed: Option<Instant>,
    reported: Instant,
}

/// Whether the loop keeps going.
enum Flow {
    Go,
    Stop,
}

/// What one turn of the decoder produced.
enum Step {
    /// A chunk this long went to the device.
    Fed(Duration),
    End,
    Failed(String),
}

impl Worker {
    pub(super) fn new(
        config: &EngineConfig,
        client: Arc<SubsonicClient>,
        runtime: tokio::runtime::Handle,
        http: reqwest::blocking::Client,
        commands: Receiver<Message>,
        replies: Sender<Message>,
        shared: Shared,
    ) -> Self {
        let Shared {
            notify,
            state: shared,
            queue: shared_queue,
        } = shared;
        let state = shared.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let error_state = Arc::clone(&shared);
        let error_notify = Arc::clone(&notify);
        // The device reports its failures from wherever it notices them,
        // which is not always this thread.
        let on_error = Arc::new(move |message: String| {
            let snapshot = {
                let mut current = error_state.lock().unwrap_or_else(|p| p.into_inner());
                current.error = Some(message);
                current.clone()
            };
            error_notify(EngineEvent::State(snapshot));
        });
        Self {
            client,
            runtime,
            http,
            commands,
            replies,
            notify,
            shared,
            shared_queue,
            output: Output::new(
                config.audio_device.clone(),
                config.buffer_ms,
                state.volume,
                on_error,
            ),
            // The equalizer, the tap and the limiter, between the decoder
            // and the device. Built at the rate the device is most likely
            // to run at and moved to its real one when it opens.
            chain: Chain::new(
                Arc::clone(&config.eq),
                Arc::clone(&config.tap),
                PREFERRED_RATE,
            ),
            normalisation: config.normalisation,
            cache: config.cache.clone(),
            state,
            open: VecDeque::new(),
            tokens: 0,
            queue: Queue::default(),
            prefetch: None,
            describing: false,
            described_failed: None,
            reported: Instant::now(),
        }
    }

    pub(super) fn run(mut self) {
        loop {
            if matches!(self.take_commands(), Flow::Stop) {
                break;
            }
            self.follow();
            // The equalizer's filters and the limiter are designed for the
            // device's rate, and the device is allowed to change under
            // them: the system default can move to a 48 kHz output while a
            // 44.1 kHz track is playing.
            if let Some(rate) = self.output.current_rate() {
                self.chain.set_rate(rate);
            }
            let worked = self.pump();
            // How far in front of the speaker everything upstream is. The
            // visualisers look back through it, so it is read every time
            // round rather than only when something is appended: it shrinks
            // as the sound plays out.
            self.chain.set_lead(self.output.ahead());
            self.report();
            if worked {
                continue;
            }
            self.prefetch();
            self.describe_one();
            let wait = if self.state.playback == Playback::Playing {
                BUSY_TICK
            } else {
                IDLE_TICK
            };
            match self.commands.recv_timeout(wait) {
                Ok(message) => {
                    if matches!(self.message(message), Flow::Stop) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
        self.output.pause();
    }

    /// Everything waiting, before any decoding: a command must not sit
    /// behind half a second of audio.
    fn take_commands(&mut self) -> Flow {
        loop {
            match self.commands.try_recv() {
                Ok(message) => {
                    if matches!(self.message(message), Flow::Stop) {
                        return Flow::Stop;
                    }
                }
                Err(TryRecvError::Disconnected) => return Flow::Stop,
                Err(TryRecvError::Empty) => return Flow::Go,
            }
        }
    }

    fn message(&mut self, message: Message) -> Flow {
        match message {
            Message::Command(command) => self.handle(command),
            Message::Described(id, song) => self.described(id, song),
            Message::Prefetched(id, opened) => self.prefetched(id, opened),
            Message::Shutdown => return Flow::Stop,
        }
        Flow::Go
    }

    /// Asks the server about one song the queue only knows the id of, so
    /// that the rows the interface draws are songs rather than blanks.
    ///
    /// The asking happens on the runtime, not here: half a second of sound
    /// is all that stands between this thread and a gap, and a queue panel
    /// is not worth risking it for. One question at a time, and a failure
    /// is left alone for a while rather than repeated every tick.
    fn describe_one(&mut self) {
        if self.describing
            || self
                .described_failed
                .is_some_and(|at| at.elapsed() < DESCRIBE_RETRY)
        {
            return;
        }
        let Some(id) = self.queue.unknown(QUEUE_LOOKAHEAD) else {
            return;
        };
        self.describing = true;
        let client = Arc::clone(&self.client);
        let replies = self.replies.clone();
        self.runtime.spawn(async move {
            let song = client.get_song(&id).await.ok();
            let _ = replies.send(Message::Described(id, Box::new(song)));
        });
    }

    fn described(&mut self, id: String, song: Box<Option<Child>>) {
        self.describing = false;
        match *song {
            Some(song) => {
                self.described_failed = None;
                self.queue.learn(&song);
                self.publish_queue();
            }
            None => {
                log::warn!("the server would not describe {id}");
                self.described_failed = Some(Instant::now());
            }
        }
    }

    /// One packet's worth of work. `false` means there was nothing to do,
    /// and the loop may wait.
    fn pump(&mut self) -> bool {
        if self.state.playback != Playback::Playing || self.open.is_empty() {
            return false;
        }
        if self.open.back().is_some_and(|open| open.drained) {
            // The end of the track being decoded. If the next one is
            // already open its sound goes into the same sink behind this
            // one, and there is nothing to hear at the join.
            if self.open.len() < 2 && self.hand_over() {
                return true;
            }
            if self.output.drained() {
                // Nothing was ready in time, so this is the ordinary path:
                // open the next track now, which is a wait on the network
                // with an empty sink — the silence P3.4 exists to avoid,
                // measured here rather than guessed at.
                let ran_out = Instant::now();
                self.advance(false);
                if self.state.playback == Playback::Playing {
                    log::info!(
                        "the join cost {} ms of silence: nothing was open in time",
                        ran_out.elapsed().as_millis()
                    );
                }
                return true;
            }
            return false;
        }
        if self.output.queued() >= TARGET_QUEUE {
            return false;
        }
        let (token, from) = {
            let open = self.open.back().expect("a track is open");
            (open.token, open.fed)
        };
        let volume = self.state.volume;
        let gain = self.open.back().expect("a track is open").gain;
        let step = {
            let open = self.open.back_mut().expect("a track is open");
            match open.decoder.next() {
                // Through the equalizer, the tap, the track's ReplayGain
                // and the limiter on the way to the device (P3.8, P3.7),
                // which is what makes the equalizer window and the
                // visualisers mean anything.
                Ok(Some(chunk)) => {
                    let shaped = self.chain.process(chunk, volume, gain);
                    match self.output.append(shaped, token, from) {
                        Ok(played) => Step::Fed(played),
                        Err(message) => Step::Failed(message),
                    }
                }
                Ok(None) => Step::End,
                Err(error) => Step::Failed(error.to_string()),
            }
        };
        match step {
            Step::Fed(played) => {
                if let Some(open) = self.open.back_mut() {
                    open.fed += played;
                }
                true
            }
            Step::End => {
                if let Some(open) = self.open.back_mut() {
                    open.drained = true;
                }
                true
            }
            Step::Failed(message) => {
                self.fail(message);
                true
            }
        }
    }

    /// What to play a song at, which is its ReplayGain when the setting is
    /// on and the server has the numbers (P3.7).
    ///
    /// Album gain while an album is playing and track gain otherwise: a
    /// record keeps its own quiet track quiet, and a shuffle across a
    /// library evens every song out. Which of the two is a question about
    /// the context, not about the song, so it is asked here rather than in
    /// the chain.
    fn gain_for(&self, song: &Child) -> f32 {
        if !self.normalisation {
            return 1.0;
        }
        let album = self
            .queue
            .snapshot()
            .context_uri
            .as_deref()
            .and_then(convert::parse_uri)
            .is_some_and(|(kind, _)| kind == convert::Kind::Album);
        let gain = replay_gain(song, album);
        if gain != 1.0 {
            log::info!(
                "playing {} at {:+.2} dB ({} gain)",
                song.title,
                20.0 * f64::from(gain).log10(),
                if album { "album" } else { "track" }
            );
        }
        gain
    }

    /// Throws away what is queued, at the device and in the tap alike: a
    /// seek, a new track, or the end of the list. What the tap holds would
    /// never have been heard, and a visualiser drawing it would be showing
    /// sound that was thrown away.
    fn restart(&mut self, playing: bool) {
        self.output.restart(playing);
        self.chain.clear();
    }

    /// Where the device has actually got to, and the join it has crossed.
    ///
    /// A join is crossed here rather than where the decoder passes it: the
    /// interface learns about a new track when the speaker reaches it, and
    /// so does the queue. Anything else would move the player bar and the
    /// queue panel half a second before the music.
    fn follow(&mut self) {
        // Only a second open track can make what is heard differ from what
        // is being decoded, and asking the device costs two locks.
        if self.open.len() < 2 {
            return;
        }
        let Some(heard) = self.output.heard() else {
            return;
        };
        while self.open.len() > 1 && self.open[0].token != heard.token {
            self.open.pop_front();
            self.crossed(heard);
        }
    }

    /// The join, once it is heard: the queue moves, and the interface is
    /// told what is playing now.
    fn crossed(&mut self, heard: Heard) {
        let Some(open) = self.open.front_mut() else {
            return;
        };
        let seam = open.seam.take();
        let track = open.track.clone();
        if seam == Some(Seam::Advance) {
            self.queue.advance(self.state.repeat);
            self.publish_queue();
        }
        log::info!("{} started without a gap", track.title);
        self.state.track = Some(track);
        self.state.position_ms = heard.position.as_millis().min(u128::from(u32::MAX)) as u32;
        self.state.position_at = (self.state.playback == Playback::Playing).then(Instant::now);
        self.publish();
    }

    /// Joins the prefetched track onto the end of the one that has just
    /// finished decoding, in the same sink. `false` if there is nothing
    /// ready to join, or if the queue changed while it was being opened and
    /// it is the wrong track now.
    fn hand_over(&mut self) -> bool {
        let wanted = self.next_up();
        let ready = match &self.prefetch {
            Some(Prefetch::Ready(ready)) => ready,
            _ => return false,
        };
        if wanted.as_ref().map(|(id, seam)| (id.as_str(), *seam))
            != Some((ready.id.as_str(), ready.seam))
        {
            // What plays next changed while this was being opened. The
            // ordinary path will open the right track.
            log::debug!("the track opened ahead of time is not the one playing next any more");
            self.prefetch = None;
            return false;
        }
        let Some(Prefetch::Ready(ready)) = self.prefetch.take() else {
            return false;
        };
        let Ready {
            song,
            decoder,
            seam,
            ..
        } = *ready;
        let token = self.token();
        let gain = self.gain_for(&song);
        self.open.push_back(Open {
            decoder,
            token,
            track: local_track(&song),
            fed: Duration::ZERO,
            drained: false,
            gain,
            // The queue moves when this is heard, not now.
            seam: Some(seam),
        });
        true
    }

    /// Opens the track that plays after this one, while this one is still
    /// playing.
    ///
    /// The opening happens on the runtime: three HTTP requests before a
    /// note (see `migration/02-audio-engine.md`) is far more than the half
    /// second of sound this thread has in hand.
    fn prefetch(&mut self) {
        // With two tracks open the join has already been made, and the one
        // after it is not decided until the queue has moved.
        if self.state.playback != Playback::Playing || self.open.len() > 1 {
            return;
        }
        let Some(open) = self.open.back() else {
            return;
        };
        let left =
            Duration::from_millis(u64::from(open.track.duration_ms)).saturating_sub(open.fed);
        if left > PREFETCH_LEAD && !open.drained {
            return;
        }
        let Some((id, seam)) = self.next_up() else {
            return;
        };
        match &self.prefetch {
            // One at a time. If the answer turns out to be the wrong
            // track, it is dropped when it arrives.
            Some(Prefetch::Opening) => return,
            Some(Prefetch::Ready(ready)) if ready.id == id && ready.seam == seam => return,
            Some(Prefetch::Failed(failed, at))
                if *failed == id && at.elapsed() < PREFETCH_RETRY =>
            {
                return;
            }
            _ => {}
        }
        // The device is open — something is playing — so this neither
        // opens it nor blocks.
        let Ok(rate) = self.output.rate() else {
            return;
        };
        // Whatever the server has already said about it, so that opening a
        // track ahead of time is not also a metadata request.
        let known = match seam {
            Seam::Again => self.queue.current(),
            Seam::Advance => self.queue.peek_next(self.state.repeat),
        }
        .and_then(|entry| entry.song.clone());
        self.prefetch = Some(Prefetch::Opening);
        let client = Arc::clone(&self.client);
        let http = self.http.clone();
        let replies = self.replies.clone();
        let cache = self.cache.clone();
        self.runtime.spawn(async move {
            let opened = match open_ahead(&client, http, cache, known, &id, rate).await {
                Ok(opened) => Some(opened),
                Err(error) => {
                    log::warn!("cannot open {id} ahead of time: {error}");
                    None
                }
            };
            let _ = replies.send(Message::Prefetched(id, Box::new(opened)));
        });
    }

    fn prefetched(&mut self, id: String, opened: Box<Option<Opened>>) {
        let Some(Opened { song, decoder }) = *opened else {
            self.prefetch = Some(Prefetch::Failed(id, Instant::now()));
            return;
        };
        // The queue may have moved on while it was being opened.
        let Some((wanted, seam)) = self.next_up().filter(|(wanted, _)| *wanted == id) else {
            log::debug!("{id} was opened ahead of time and is no longer what plays next");
            self.prefetch = None;
            return;
        };
        // A track queued by hand arrives as an id, and opening it is also
        // the answer to what it is — worth telling the queue panel about,
        // and only then.
        let learned = self
            .queue
            .peek_next(self.state.repeat)
            .is_some_and(|entry| entry.song.is_none());
        self.queue.learn(&song);
        if learned {
            self.publish_queue();
        }
        self.prefetch = Some(Prefetch::Ready(Box::new(Ready {
            id: wanted,
            song,
            decoder,
            seam,
        })));
    }

    /// What plays after the track being decoded, and what the queue does
    /// when it starts.
    ///
    /// Only ever asked with one track open, which is the state where the
    /// track being decoded is also the queue's current one — with two open
    /// the join has been made and the queue has not caught up yet.
    fn next_up(&self) -> Option<(String, Seam)> {
        if self.state.repeat == RepeatMode::Track {
            return self
                .queue
                .current()
                .map(|entry| (entry.id.clone(), Seam::Again));
        }
        self.queue
            .peek_next(self.state.repeat)
            .map(|entry| (entry.id.clone(), Seam::Advance))
    }

    /// A tag for a track's sound, different from the last one.
    fn token(&mut self) -> Token {
        self.tokens = self.tokens.wrapping_add(1);
        self.tokens
    }

    /// Corrects the position the interface is interpolating from.
    fn report(&mut self) {
        if self.state.playback != Playback::Playing || self.open.is_empty() {
            return;
        }
        if self.reported.elapsed() < POSITION_REPORT {
            return;
        }
        let position = self.position();
        self.state.position_ms = position;
        self.state.position_at = Some(Instant::now());
        self.publish();
    }

    /// Where the music is: what the device has taken of the track the
    /// interface believes is playing.
    ///
    /// A position from the track *after* it — the sound of a join that
    /// [`Worker::follow`] has not caught up with yet — is not reported,
    /// because it would be read as the current track's.
    fn position(&mut self) -> u32 {
        let playing = self.open.front().map(|open| open.token);
        match self.output.heard() {
            Some(heard) if Some(heard.token) == playing => {
                heard.position.as_millis().min(u128::from(u32::MAX)) as u32
            }
            _ => self.state.position_ms,
        }
    }

    fn handle(&mut self, command: PlayerCommand) {
        match command {
            PlayerCommand::Toggle => self.toggle(),
            PlayerCommand::Next => self.advance(true),
            PlayerCommand::Previous => self.previous(),
            // Rule 7: Clear empties your part of the queue and leaves
            // the album's rows alone.
            PlayerCommand::ClearQueue => {
                self.queue.clear_queued();
                self.publish_queue();
            }
            PlayerCommand::AddToQueue(uri) => {
                if let Some(id) = convert::id_of(&uri, convert::Kind::Track) {
                    self.queue.add(Entry::new(id));
                    self.publish_queue();
                }
            }
            PlayerCommand::PlayQueued(row) => self.play_queued(row),
            PlayerCommand::Seek(position_ms) => self.seek(position_ms),
            // Local playback makes these the same thing: there is no
            // Connect round trip for the preview to avoid.
            PlayerCommand::Volume(volume) | PlayerCommand::VolumePreview(volume) => {
                self.output.set_volume(volume);
                self.state.volume = volume;
                self.publish();
            }
            PlayerCommand::Shuffle(shuffle) => {
                self.queue.set_shuffle(shuffle);
                self.state.shuffle = shuffle;
                self.publish();
                self.publish_queue();
            }
            PlayerCommand::Repeat(repeat) => {
                self.state.repeat = repeat;
                self.publish();
            }
            PlayerCommand::Load(spec) => self.load(spec),
            // Connect's "play here instead". There is nowhere else.
            PlayerCommand::Activate => {}
        }
    }

    fn toggle(&mut self) {
        match self.state.playback {
            Playback::Playing => {
                self.output.pause();
                self.state.position_ms = self.position();
                self.state.position_at = None;
                self.state.playback = Playback::Paused;
                self.publish();
            }
            Playback::Paused => {
                self.output.play();
                self.state.position_ms = self.position();
                self.state.position_at = Some(Instant::now());
                self.state.playback = Playback::Playing;
                self.publish();
            }
            Playback::Stopped => self.restart_after_stop(),
            Playback::Loading => {}
        }
    }

    fn seek(&mut self, position_ms: u32) {
        // The track the interface is asking about is the one being heard,
        // which is the front of the list. Anything joined onto the end of
        // it is sound from after the jump and goes with the rest.
        self.open.truncate(1);
        let Some(current) = self.open.front() else {
            return;
        };
        // A reader that has read to the end of its stream cannot always
        // seek back out of it — symphonia's MP4 reader answers "no atom
        // pending read" — and the end of a track is exactly where repeat
        // one asks for a seek. So the track is opened again instead.
        if current.drained {
            self.state.seek_sequence = self.state.seek_sequence.wrapping_add(1);
            let play = self.state.playback != Playback::Paused;
            self.start(play, position_ms);
            return;
        }
        let playing = self.state.playback == Playback::Playing;
        let target = Duration::from_millis(u64::from(position_ms));
        // The sink is emptied first: what is queued is the audio from
        // before the jump, and it would be heard after it.
        self.restart(playing);
        let landed = match self
            .open
            .front_mut()
            .expect("a track is open")
            .decoder
            .seek(target)
        {
            Ok(landed) => landed,
            Err(error) => return self.fail(error.to_string()),
        };
        if let Some(current) = self.open.front_mut() {
            current.fed = landed;
            current.drained = false;
        }
        self.state.position_ms = landed.as_millis() as u32;
        self.state.position_at = playing.then(Instant::now);
        self.state.seek_sequence = self.state.seek_sequence.wrapping_add(1);
        self.publish();
    }

    /// The track after this one, either because this one ended or because
    /// Next was pressed. Repeat one applies only to the first case: asking
    /// for the next track and getting the same one again is not what the
    /// button says.
    fn advance(&mut self, manual: bool) {
        if self.queue.is_empty() {
            return;
        }
        if !manual && self.state.repeat == RepeatMode::Track && self.queue.current().is_some() {
            self.seek(0);
            return;
        }
        let play = self.state.playback != Playback::Paused;
        if self.queue.advance(self.state.repeat) {
            // Rule 4: the row goes now, not once the track has opened.
            self.publish_queue();
            self.start(play, 0);
        } else {
            self.finish();
        }
    }

    fn previous(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        let position = Duration::from_millis(u64::from(self.position()));
        match self.queue.previous(self.state.repeat, position) {
            Rewound::Moved => {
                self.publish_queue();
                self.start(self.state.playback != Playback::Paused, 0);
            }
            Rewound::Restart => self.seek(0),
        }
    }

    /// Rule 5: playing a row of the queue skips down to it.
    fn play_queued(&mut self, row: usize) {
        if !self.queue.skip_to(row) {
            return;
        }
        let play = self.state.playback != Playback::Paused;
        self.publish_queue();
        self.start(play, 0);
    }

    /// Play, after playback stopped. A row the queue is still on is the one
    /// that failed, so it is tried again; otherwise the list starts from
    /// the top, because a list that has run out is wound back rather than
    /// emptied.
    fn restart_after_stop(&mut self) {
        if self.queue.current().is_some() {
            self.start(true, 0);
        } else if self.queue.advance(self.state.repeat) {
            self.publish_queue();
            self.start(true, 0);
        }
    }

    /// The end of the list, with nothing to repeat. The list stays, wound
    /// back to its start: pressing play after an album has run out plays
    /// the album, which is what the button appears to promise.
    fn finish(&mut self) {
        self.open.clear();
        self.queue.rewind();
        self.restart(false);
        self.state.playback = Playback::Stopped;
        self.state.position_ms = 0;
        self.state.position_at = None;
        self.publish();
        self.publish_queue();
    }

    fn load(&mut self, spec: LoadSpec) {
        let entries = match self.expand(&spec) {
            Ok(entries) => entries,
            Err(message) => return self.fail(message),
        };
        if entries.is_empty() {
            return self.fail("There is nothing to play here.".into());
        }
        // Before the load, so that a shuffled play is shuffled from its
        // first track rather than from its second.
        if let Some(shuffle) = spec.shuffle {
            self.queue.set_shuffle(shuffle);
            self.state.shuffle = shuffle;
        }
        let start = start_index(&entries, spec.offset_uri.as_deref(), spec.offset_index);
        // Rule 6: the songs queued by hand survive a new album.
        self.queue.load(spec.context_uri.clone(), entries, start);
        // Rule 9: a restore carries the queue with it — the songs owed to
        // "Playing next", and the one that was playing over the top of the
        // album, which leaves the album where the offset put it.
        for uri in &spec.queued {
            if let Some(id) = convert::id_of(uri, convert::Kind::Track) {
                self.queue.add(Entry::new(id));
            }
        }
        if let Some(id) = spec
            .current
            .as_deref()
            .and_then(|uri| convert::id_of(uri, convert::Kind::Track))
        {
            self.queue.play_queued_now(Entry::new(id));
        }
        self.publish_queue();
        self.start(spec.play, spec.position_ms);
    }

    /// What a load asked for, as a list of song ids — with whatever the
    /// server already told us about each one, so that playing an album is
    /// one request rather than one per track.
    fn expand(&self, spec: &LoadSpec) -> Result<Vec<Entry>, String> {
        if !spec.uris.is_empty() {
            return Ok(spec
                .uris
                .iter()
                .filter_map(|uri| convert::id_of(uri, convert::Kind::Track))
                .map(Entry::new)
                .collect());
        }
        let Some(context) = spec.context_uri.as_deref() else {
            return Err("There is nothing to play here.".into());
        };
        let (kind, id) = convert::parse_uri(context)
            .ok_or_else(|| format!("{context} is not something this app can play."))?;
        let songs = match kind {
            convert::Kind::Album => {
                self.runtime
                    .block_on(self.client.get_album(id))
                    .map_err(|error| error.to_string())?
                    .song
            }
            convert::Kind::Playlist => {
                self.runtime
                    .block_on(self.client.get_playlist(id))
                    .map_err(|error| error.to_string())?
                    .entry
            }
            convert::Kind::Track => vec![
                self.runtime
                    .block_on(self.client.get_song(id))
                    .map_err(|error| error.to_string())?,
            ],
            convert::Kind::Artist => self.artist_songs(id)?,
            // The Liked Songs page. One request answers the whole list,
            // which is why it is a context rather than a list of URIs.
            convert::Kind::Collection => {
                self.runtime
                    .block_on(self.client.starred())
                    .map_err(|error| error.to_string())?
                    .song
            }
        };
        Ok(songs.into_iter().map(Entry::known).collect())
    }

    /// Playing an artist: their popular songs if the server can rank them,
    /// and otherwise their records, oldest first.
    ///
    /// `getTopSongs` is Last.fm-backed, so on the ordinary self-hosted
    /// server with no key it answers with nothing — which is not an error,
    /// just an artist page that has no Popular section. The albums are what
    /// makes the Play button on that page mean something anyway.
    fn artist_songs(&self, id: &str) -> Result<Vec<Child>, String> {
        let artist = self
            .runtime
            .block_on(self.client.get_artist(id))
            .map_err(|error| error.to_string())?;
        let top = self
            .runtime
            .block_on(self.client.top_songs(&artist.name, 100))
            .unwrap_or_default();
        if !top.is_empty() {
            return Ok(top);
        }
        let mut albums = artist.album.clone();
        albums.sort_by_key(|album| (album.year, album.name.clone()));
        let mut songs = Vec::new();
        for album in &albums {
            match self.runtime.block_on(self.client.get_album(&album.id)) {
                Ok(album) => songs.extend(album.song),
                Err(error) => log::warn!("cannot read {}: {error}", album.name),
            }
        }
        if songs.is_empty() {
            return Err(format!("{} has nothing to play.", artist.name));
        }
        Ok(songs)
    }

    /// Opens the track the queue is on and starts it.
    ///
    /// This is the path every command takes, and the one a join falls back
    /// to when nothing was ready in time: it empties the sink and waits for
    /// the track to open, which is the gap P3.4 exists to avoid. A track
    /// that was already opened ahead of time is used instead of being
    /// opened again, which is what makes pressing Next quick as well.
    fn start(&mut self, play: bool, position_ms: u32) {
        self.open.clear();
        self.state.playback = Playback::Loading;
        self.state.position_ms = position_ms;
        self.state.position_at = None;
        self.state.error = None;
        self.publish();

        let song = match self.song() {
            Ok(song) => song,
            Err(message) => return self.fail(message),
        };
        self.state.track = Some(local_track(&song));
        self.publish();

        // Opening the device first: its rate is what the track decodes to.
        let rate = match self.output.rate() {
            Ok(rate) => rate,
            Err(message) => return self.fail(message),
        };
        // The chain follows the same rate; the loop reads it back from the
        // device every time round, so this is only about the first chunk
        // after the device is opened.
        self.chain.set_rate(rate);
        self.restart(play);
        let mut decoder = match self.take_ready(&song.id, position_ms) {
            Some(decoder) => decoder,
            None => {
                let url = match self.client.stream_url(&song.id) {
                    Ok(url) => url,
                    Err(error) => return self.fail(error.to_string()),
                };
                match Decoder::open(
                    self.http.clone(),
                    stream_of(&song, url, self.cache.clone()),
                    rate,
                ) {
                    Ok(decoder) => decoder,
                    Err(error) => return self.fail(error.to_string()),
                }
            }
        };
        let mut from = Duration::ZERO;
        if position_ms > 0 {
            match decoder.seek(Duration::from_millis(u64::from(position_ms))) {
                Ok(landed) => from = landed,
                Err(error) => log::warn!("cannot start {} part way in: {error}", song.id),
            }
        }
        log::info!(
            "playing {} as {} ({} request(s) to open it)",
            song.title,
            decoder.codec(),
            decoder.requests()
        );
        let token = self.token();
        let gain = self.gain_for(&song);
        self.open.push_back(Open {
            decoder,
            token,
            track: local_track(&song),
            fed: from,
            drained: false,
            gain,
            // Whatever the queue does, it has done already: a command is
            // what brought us here.
            seam: None,
        });
        self.state.playback = if play {
            Playback::Playing
        } else {
            Playback::Paused
        };
        self.state.position_ms = from.as_millis() as u32;
        self.state.position_at = play.then(Instant::now);
        self.publish();
    }

    /// The decoder for this track if it was opened ahead of time, and only
    /// from the top: a prefetched track has not been decoded from, so it is
    /// at its first packet and nothing else.
    fn take_ready(&mut self, id: &str, position_ms: u32) -> Option<Decoder> {
        if position_ms > 0 {
            return None;
        }
        let Some(Prefetch::Ready(ready)) = &self.prefetch else {
            return None;
        };
        if ready.id != id {
            return None;
        }
        let Some(Prefetch::Ready(ready)) = self.prefetch.take() else {
            return None;
        };
        Some(ready.decoder)
    }

    /// The server's record of the track being played, asked for once. This
    /// one is worth waiting for: nothing can play without it.
    fn song(&mut self) -> Result<Child, String> {
        let entry = self
            .queue
            .current()
            .ok_or_else(|| "There is nothing to play here.".to_string())?;
        if let Some(song) = &entry.song {
            return Ok(song.clone());
        }
        let id = entry.id.clone();
        let song = self
            .runtime
            .block_on(self.client.get_song(&id))
            .map_err(|error| error.to_string())?;
        self.queue.learn_current(song.clone());
        Ok(song)
    }

    /// Playback stops, and the interface says why.
    fn fail(&mut self, message: String) {
        log::error!("{message}");
        self.open.clear();
        self.restart(false);
        self.state.playback = Playback::Stopped;
        self.state.position_at = None;
        self.state.error = Some(message);
        self.publish();
    }

    fn publish(&mut self) {
        *self.shared.lock().unwrap_or_else(|p| p.into_inner()) = self.state.clone();
        (self.notify)(EngineEvent::State(self.state.clone()));
        self.reported = Instant::now();
    }

    /// The queue, pushed separately from the state: it is a list rather
    /// than a line, and most of what happens to playback leaves it alone.
    fn publish_queue(&mut self) {
        let snapshot = self.queue.snapshot();
        *self.shared_queue.lock().unwrap_or_else(|p| p.into_inner()) = snapshot.clone();
        (self.notify)(EngineEvent::Queue(snapshot));
    }
}

/// Opens a track away from the audio thread, for the join it is going to
/// be needed at.
///
/// The metadata call is asynchronous and the stream is not: the reader is a
/// blocking one, and `spawn_blocking` is where a blocking read belongs so
/// that it does not sit on one of the runtime's workers.
async fn open_ahead(
    client: &SubsonicClient,
    http: reqwest::blocking::Client,
    cache: Option<Arc<Cache>>,
    known: Option<Child>,
    id: &str,
    rate: u32,
) -> anyhow::Result<Opened> {
    let song = match known {
        Some(song) => song,
        None => client.get_song(id).await?,
    };
    let url = client.stream_url(&song.id)?;
    tokio::task::spawn_blocking(move || {
        let decoder = Decoder::open(http, stream_of(&song, url, cache), rate)?;
        Ok(Opened { song, decoder })
    })
    .await?
}

/// One track for the decoder to open: the server's description of it, and
/// where its bytes may be kept.
fn stream_of(song: &Child, url: String, cache: Option<Arc<Cache>>) -> Stream {
    Stream {
        url,
        id: song.id.clone(),
        // A negative size is a server saying something impossible; the
        // cache then has nothing to check its copy against and says so
        // rather than believing it.
        size: song.size.and_then(|size| u64::try_from(size).ok()),
        suffix: song.suffix.clone(),
        mime: song.content_type.clone(),
        cache,
    }
}

/// Where a load starts: the track it named, else the index it named, else
/// the beginning.
fn start_index(entries: &[Entry], offset_uri: Option<&str>, offset_index: Option<u32>) -> usize {
    if let Some(id) = offset_uri.and_then(|uri| convert::id_of(uri, convert::Kind::Track))
        && let Some(found) = entries.iter().position(|entry| entry.id == id)
    {
        return found;
    }
    offset_index
        .map(|index| index as usize)
        .filter(|index| *index < entries.len())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(ids: &[&str]) -> Vec<Entry> {
        ids.iter().map(|id| Entry::new(*id)).collect()
    }

    #[test]
    fn a_load_starts_where_it_was_asked_to() {
        let list = entries(&["a", "b", "c"]);
        assert_eq!(start_index(&list, Some("sonic:track:c"), None), 2);
        assert_eq!(start_index(&list, None, Some(1)), 1);
        // A track that is not in the list, and an index past its end, both
        // fall back to the beginning rather than to nothing.
        assert_eq!(start_index(&list, Some("sonic:track:z"), None), 0);
        assert_eq!(start_index(&list, None, Some(9)), 0);
        assert_eq!(start_index(&list, None, None), 0);
    }
}

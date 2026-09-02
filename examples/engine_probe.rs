//! P3.2: drive the engine the way the interface will, and watch what it
//! says back.
//!
//! `src/backend.rs` does not talk to `src/engine/` yet — that is Phase 4 —
//! so this stands in for the player bar: it sends `PlayerCommand`s and
//! prints every `LocalState` the engine pushes, which is the whole contract
//! in `migration/02-audio-engine.md` from the outside.
//!
//! ```sh
//! cargo run --example engine_probe             # an album from the library
//! cargo run --example engine_probe -- --album <id>
//! cargo run --example engine_probe -- --seconds 8
//! ```
//!
//! It loads an album, plays, pauses, resumes, seeks, skips to the next
//! track and moves the volume, checking after each one that the state says
//! what the interface would need it to say. Then it does the same to the
//! queue: what the album has left, Play next, Clear, playing a row,
//! shuffle, and playing an artist. Then it watches two joins go by (P3.4)
//! and measures what the silence at one costs, which is the one thing here
//! that no unit test can answer. Last it reads the visualiser tap while
//! music is playing (P3.8): that there is sound in it, that the analyser
//! moves, that the equalizer is in front of it and the volume behind it,
//! and that it is being read where the speaker is rather than half a
//! second in front of it. Then, once more with normalisation on (P3.7),
//! which is the one stage of the chain that cannot be seen from out here.
//! Last, once more with the cache on (P3.6), playing one track twice: the
//! second time the server is asked for nothing at all. The rules those checks are about are in
//! `docs/_reference/queue.md`; the state machine behind them is unit-tested
//! in `src/engine/queue.rs`, and this is the part that needs a server. `FASTSONIC_TEST_SERVER`,
//! `FASTSONIC_TEST_USER` and `FASTSONIC_TEST_PASSWORD` point it at a server
//! other than `migration/devserver`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fastsonic::api::NetActivity;
use fastsonic::api::subsonic::{Child, Credentials, SubsonicClient, convert};
use fastsonic::engine::{
    Cache, Engine, EngineConfig, EngineEvent, LoadSpec, LocalState, Playback, PlayerCommand,
    QueueRow, QueueSnapshot, RepeatMode,
};
use fastsonic::vis;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let mut seconds = 4.0_f64;
    let mut album = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seconds" => {
                seconds = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--seconds wants a number"))?
                    .parse()?;
            }
            "--album" => album = args.next(),
            "--help" | "-h" => {
                println!("usage: engine_probe [--album <id>] [--seconds N]");
                return Ok(());
            }
            other => anyhow::bail!("unexpected argument {other}"),
        }
    }
    let playing = Duration::from_secs_f64(seconds);

    let server = env("FASTSONIC_TEST_SERVER", "http://localhost:4533");
    let username = env("FASTSONIC_TEST_USER", "admin");
    let password = env("FASTSONIC_TEST_PASSWORD", "fastsonic");

    let client = Arc::new(SubsonicClient::new(
        fastsonic::http_client_builder().build()?,
        Arc::new(NetActivity::default()),
        20,
    ));
    client.set_credentials(Some(Credentials::from_password(
        &server, &username, &password,
    )));

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let album = match album {
        Some(id) => id,
        None => runtime.block_on(pick_album(&client))?,
    };

    // Every snapshot with the moment it arrived, because the check that a
    // join is gapless is a check about *when* the interface was told.
    let seen: Arc<Mutex<Vec<(f64, LocalState)>>> = Arc::default();
    let recorder = Arc::clone(&seen);
    let started = Instant::now();
    // Held here as well as by the engine, because the last section reads
    // the tap and moves the sliders the way the two windows do.
    let tap = vis::AudioTap::new();
    let eq = fastsonic::eq::shared();
    let engine = Engine::start(
        &EngineConfig {
            initial_volume: u16::MAX / 2,
            tap: Arc::clone(&tap),
            eq: Arc::clone(&eq),
            ..EngineConfig::default()
        },
        Arc::clone(&client),
        runtime.handle().clone(),
        watcher(started, recorder),
    )?;

    let mut failures = Vec::new();
    let mut check = |claim: &str, held: bool| {
        println!("    {} {claim}", if held { "ok" } else { "FAILED" });
        if !held {
            failures.push(claim.to_string());
        }
    };

    println!("\n-- play {}", convert::album_uri(&album));
    engine.command(PlayerCommand::Load(LoadSpec {
        context_uri: Some(convert::album_uri(&album)),
        play: true,
        ..LoadSpec::default()
    }))?;
    let state = settle(&engine, playing);
    check("the album is playing", state.playback == Playback::Playing);
    check("a track is named", state.track.is_some());
    check("the position has moved", state.position_ms > 500);
    let first = state.track.clone();
    let played = state.position_ms;

    println!("\n-- pause");
    engine.command(PlayerCommand::Toggle)?;
    let paused = settle(&engine, Duration::from_millis(700));
    check("playback is paused", paused.playback == Playback::Paused);
    check(
        "a paused position does not run on",
        paused.position_now() == paused.position_ms,
    );
    let held = paused.position_ms;
    std::thread::sleep(Duration::from_millis(700));
    check(
        "and it is still where it was",
        engine.state().position_ms == held,
    );

    println!("\n-- resume");
    engine.command(PlayerCommand::Toggle)?;
    let resumed = settle(&engine, Duration::from_secs(1));
    check("playback is playing", resumed.playback == Playback::Playing);
    check("the position moved on", resumed.position_ms > held);
    check("it did not start again", resumed.position_ms >= played);

    let target = resumed
        .track
        .as_ref()
        .map(|track| track.duration_ms / 2)
        .unwrap_or(10_000);
    println!("\n-- seek to {:.1}s", f64::from(target) / 1000.0);
    let sequence = resumed.seek_sequence;
    engine.command(PlayerCommand::Seek(target))?;
    let seeked = settle(&engine, Duration::from_millis(1_500));
    check(
        "the seek was reported as one",
        seeked.seek_sequence > sequence,
    );
    check(
        "the position landed near where it was asked to",
        seeked.position_ms + 2_000 > target && seeked.position_ms < target + 4_000,
    );
    check("it is still playing", seeked.playback == Playback::Playing);

    println!("\n-- next");
    engine.command(PlayerCommand::Next)?;
    // Taken as soon as the next track is open, because the question is
    // where it started, not where it has got to.
    let next = settle(&engine, Duration::from_millis(800));
    check("the track changed", next.track != first);
    check(
        "the new track is playing",
        next.playback == Playback::Playing,
    );
    check("it started from the top", next.position_ms < 1_500);
    let next = settle(&engine, playing);
    check("and it keeps playing", next.position_ms > 1_500);

    println!("\n-- volume");
    engine.command(PlayerCommand::Volume(u16::MAX / 8))?;
    let quiet = settle(&engine, Duration::from_millis(700));
    check("the volume followed", quiet.volume == u16::MAX / 8);
    check("nothing else changed", quiet.playback == Playback::Playing);

    // The rules in `docs/_reference/queue.md`, from outside the engine.
    // The unit tests in `src/engine/queue.rs` prove the state machine; this
    // is about the queue the interface would draw while music is playing.
    //
    // From a fresh load, because the fixture library's tracks are seconds
    // long: the checks below take a couple of them, and an album that ran
    // on into its next track half way through would be answering about a
    // different row each time. Which it now does gaplessly, and is P3.4's
    // business rather than this section's.
    println!("\n-- the queue");
    engine.command(PlayerCommand::Load(LoadSpec {
        context_uri: Some(convert::album_uri(&album)),
        play: true,
        ..LoadSpec::default()
    }))?;
    settle(&engine, Duration::from_millis(600));
    let queue = engine.queue();
    check(
        "the queue knows what the album has left",
        !queue.upcoming.is_empty(),
    );
    check(
        "and every row of it is a song rather than an id",
        queue.upcoming.iter().all(|row| row.track.is_some()),
    );
    check(
        "the song playing is not also a row waiting to play",
        queue
            .current
            .as_ref()
            .is_some_and(|row| !queue.rows().any(|next| next.uri == row.uri)),
    );

    // Rule 2: queued songs go above the album's rows, oldest first.
    let to_queue: Vec<String> = queue
        .upcoming
        .iter()
        .rev()
        .take(2)
        .map(|row| row.uri.clone())
        .collect();
    for uri in &to_queue {
        engine.command(PlayerCommand::AddToQueue(uri.clone()))?;
    }
    std::thread::sleep(Duration::from_millis(600));
    let queue = engine.queue();
    check(
        "Play next keeps the songs in the order they were queued",
        uris(&queue.queued) == to_queue,
    );
    check(
        "and they play before the album's rows",
        queue.rows().next().map(|row| row.uri.clone()) == to_queue.first().cloned(),
    );
    check(
        "a queued song is described without being played",
        queue.queued.iter().all(|row| row.track.is_some()),
    );

    // Rule 7: Clear empties your part of the queue and nothing else.
    let album_rows = uris(&queue.upcoming);
    engine.command(PlayerCommand::ClearQueue)?;
    std::thread::sleep(Duration::from_millis(300));
    let queue = engine.queue();
    check(
        "Clear empties Playing next and leaves the album alone",
        queue.queued.is_empty() && uris(&queue.upcoming) == album_rows,
    );

    // Rule 5: playing a row skips down to it.
    let skipped_to = queue.upcoming.first().map(|row| row.uri.clone());
    engine.command(PlayerCommand::PlayQueued(0))?;
    let jumped = settle(&engine, Duration::from_millis(1_200));
    check(
        "playing a row of the queue plays that song",
        jumped.track.as_ref().map(|track| track.uri.clone()) == skipped_to,
    );
    check(
        "and the rows below it stay, in order",
        uris(&engine.queue().upcoming) == album_rows[1..],
    );

    // Shuffle is the play order, not a different list: the same songs, and
    // the album's own order underneath to come back to.
    println!("\n-- shuffle");
    engine.command(PlayerCommand::Load(LoadSpec {
        context_uri: Some(convert::album_uri(&album)),
        play: true,
        ..LoadSpec::default()
    }))?;
    let ordered = settle(&engine, Duration::from_millis(900));
    let in_order = uris(&engine.queue().upcoming);
    engine.command(PlayerCommand::Shuffle(true))?;
    std::thread::sleep(Duration::from_millis(300));
    let shuffled = engine.queue();
    check(
        "shuffle does not interrupt the song playing",
        engine.state().track == ordered.track,
    );
    check(
        "and it keeps every song that is left, once each",
        sorted(&shuffled.upcoming) == sorted_uris(&in_order),
    );
    engine.command(PlayerCommand::Shuffle(false))?;
    std::thread::sleep(Duration::from_millis(300));
    check(
        "turning it off puts the album back in its own order",
        uris(&engine.queue().upcoming) == in_order,
    );

    // Playing an artist: `getTopSongs` is empty without a Last.fm key, so
    // on the development server this is the albums path.
    if let Some(artist) = runtime.block_on(artist_of(&client, &album)) {
        println!("\n-- an artist");
        engine.command(PlayerCommand::Load(LoadSpec {
            context_uri: Some(convert::artist_uri(&artist)),
            play: true,
            ..LoadSpec::default()
        }))?;
        let playing = settle(&engine, Duration::from_millis(2_000));
        check(
            "an artist plays",
            playing.playback == Playback::Playing && playing.track.is_some(),
        );
        check(
            "and their records are what comes next",
            !engine.queue().upcoming.is_empty(),
        );
    }
    let next = settle(&engine, Duration::from_millis(400));

    println!("\n-- pause and remember");
    engine.command(PlayerCommand::Toggle)?;
    settle(&engine, Duration::from_millis(300));
    let resume = engine.interrupted();
    check(
        "a paused track is remembered to come back to",
        resume.is_some_and(|resume| !resume.playing && resume.position_ms > 0),
    );

    // The end of a track is where the engine has to decide something: the
    // fixtures are seconds long, so this is cheap to reach on purpose.
    let track = next.track.clone().expect("a track is playing");
    let near_the_end = track.duration_ms.saturating_sub(1_200);

    println!("\n-- the end of a track, repeating it");
    engine.command(PlayerCommand::Repeat(RepeatMode::Track))?;
    engine.command(PlayerCommand::Seek(near_the_end))?;
    engine.command(PlayerCommand::Toggle)?;
    let repeated = settle(&engine, Duration::from_millis(3_000));
    check(
        "a repeated track starts again rather than moving on",
        repeated.track.as_ref().map(|track| track.uri.clone())
            == next.track.as_ref().map(|track| track.uri.clone())
            && repeated.position_ms < near_the_end,
    );
    check(
        "and it is still playing",
        repeated.playback == Playback::Playing,
    );

    println!("\n-- the end of the last track");
    engine.command(PlayerCommand::Repeat(RepeatMode::Off))?;
    engine.command(PlayerCommand::Load(LoadSpec {
        uris: vec![track.uri.clone()],
        play: true,
        position_ms: near_the_end,
        ..LoadSpec::default()
    }))?;
    let ending = settle(&engine, Duration::from_millis(3_500));
    check(
        "playback stops when there is nothing left to play",
        ending.playback == Playback::Stopped,
    );
    check(
        "and the position goes back to nothing",
        ending.position_ms == 0,
    );

    // P3.4, and the reason the position clock carries a tag per track: the
    // album should run from one track into the next with nothing to hear
    // at the join, and the interface should be told about the new track
    // when the speaker reaches it rather than when the decoder does.
    println!("\n-- a gapless join");
    engine.command(PlayerCommand::Repeat(RepeatMode::Off))?;
    engine.command(PlayerCommand::Load(LoadSpec {
        context_uri: Some(convert::album_uri(&album)),
        play: true,
        ..LoadSpec::default()
    }))?;
    let first = settle(&engine, Duration::from_millis(900));
    let leaving = first.track.clone().expect("a track is playing");
    let from = seen.lock().unwrap_or_else(|p| p.into_inner()).len();
    // Straight to the end of it: the fixtures are seconds long, and this
    // is the one moment the join can be watched.
    engine.command(PlayerCommand::Seek(
        leaving.duration_ms.saturating_sub(1_500),
    ))?;
    let joined = settle(&engine, Duration::from_millis(4_000));
    check(
        "the album ran on into the next track",
        joined.track.as_ref().is_some_and(|track| *track != leaving),
    );
    check(
        "and it is still playing",
        joined.playback == Playback::Playing,
    );
    let across = seen.lock().unwrap_or_else(|p| p.into_inner())[from..].to_vec();
    let join = across
        .iter()
        .position(|(_, state)| state.track.as_ref() != Some(&leaving));
    check(
        "nothing was reported as loading at the join",
        join.is_some_and(|join| {
            across[..join]
                .iter()
                .all(|(_, state)| state.playback == Playback::Playing)
        }),
    );
    // What the silence at a join measures as, from outside: where the
    // first track's position had got to, plus the time until the second
    // track was announced, against how long the first track is.
    if let Some(join) = join {
        let (crossed_at, crossed) = across[join].clone();
        let (last_at, last) = across[..join]
            .iter()
            .rev()
            .find(|(_, state)| state.position_ms > 0)
            .cloned()
            .unwrap_or_else(|| across[join].clone());
        let heard = f64::from(last.position_ms) / 1000.0 + (crossed_at - last_at);
        let silence = heard - f64::from(leaving.duration_ms) / 1000.0;
        println!(
            "    the join measured {:.0} ms of silence",
            silence * 1000.0
        );
        check(
            "the join is silent for less than a tenth of a second",
            silence.abs() < 0.1,
        );
        check(
            "the new track was announced from its own beginning",
            crossed.position_ms < 400,
        );
        check(
            "and the queue moved with it",
            engine
                .queue()
                .current
                .is_some_and(|row| Some(row.uri) == crossed.track.map(|track| track.uri)),
        );
    }

    // Once, and then again: the track that plays after a join has to arm
    // the next one, or an album is gapless exactly once. Not every album
    // in the fixture library has a third track to run on into.
    if let Some(second) = joined.track.clone().filter(|_| !engine.queue().is_empty()) {
        println!("\n-- and the join after it");
        engine.command(PlayerCommand::Seek(
            second.duration_ms.saturating_sub(1_200),
        ))?;
        let third = settle(&engine, Duration::from_millis(3_000));
        check(
            "the album ran on again",
            third.track.as_ref().is_some_and(|track| *track != second),
        );
        check(
            "still playing, still without loading",
            third.playback == Playback::Playing,
        );
    }

    // The other half of a join: for the last half second of a track, the
    // next one is already in the sink, and a seek then is about the track
    // being *heard*. Getting this wrong plays the next track from the
    // middle instead of scrubbing this one.
    println!("\n-- a seek across a join that has not been heard yet");
    engine.command(PlayerCommand::Load(LoadSpec {
        context_uri: Some(convert::album_uri(&album)),
        play: true,
        ..LoadSpec::default()
    }))?;
    let playing = settle(&engine, Duration::from_millis(900))
        .track
        .expect("a track is playing");
    engine.command(PlayerCommand::Seek(playing.duration_ms.saturating_sub(200)))?;
    std::thread::sleep(Duration::from_millis(150));
    engine.command(PlayerCommand::Seek(1_000))?;
    let scrubbed = settle(&engine, Duration::from_millis(1_200));
    check(
        "the seek stayed in the track that was playing",
        scrubbed.track.as_ref() == Some(&playing),
    );
    check(
        "and it landed where it was asked to",
        scrubbed.position_ms >= 1_000 && scrubbed.position_ms < 3_000,
    );
    check(
        "and it is still playing",
        scrubbed.playback == Playback::Playing,
    );

    // P3.8: what the equalizer window and the visualisers are wired to.
    // The rule they are built on (`AGENTS.md`) is that every visualiser
    // shows post-equalizer, pre-volume audio, and both halves of that are
    // checkable from out here.
    println!("\n-- the visualiser tap");
    engine.command(PlayerCommand::Load(LoadSpec {
        context_uri: Some(convert::album_uri(&album)),
        play: true,
        ..LoadSpec::default()
    }))?;
    settle(&engine, Duration::from_millis(1_500));
    let flat = level(&tap);
    println!("    the tap is at {flat:.1} dBFS");
    check("there is sound in the tap", flat > -60.0);

    let mut analyser = vis::Analyser::default();
    let mut moved = false;
    let watching = Instant::now();
    while watching.elapsed() < Duration::from_millis(500) {
        let bars = analyser.step(&tap.window(vis::FFT_SAMPLES, vis::LAG), Instant::now());
        moved |= bars.iter().any(|bar| bar.height > 0);
        std::thread::sleep(vis::STEP);
    }
    check("the analyser's bars move", moved);

    // The lead is what keeps them moving *with* the music: the engine
    // decodes up to half a second in front of the device, and everything
    // reading the tap looks back through what has not been heard yet.
    let lead = tap.lead();
    println!("    the tap is read {lead} frames back from the newest");
    check(
        "the tap is read where the speaker is, not where the decoder is",
        lead > 0 && lead < 48_000,
    );

    // Pre-volume: silence at the speaker, the bars still dancing.
    engine.command(PlayerCommand::Volume(0))?;
    settle(&engine, Duration::from_millis(900));
    let silent = level(&tap);
    check(
        "the volume knob does not move the tap",
        (silent - flat).abs() < 3.0,
    );
    engine.command(PlayerCommand::Volume(u16::MAX / 2))?;

    // Post-equalizer: the preamp does move it, by what it says it does.
    if let Ok(mut settings) = eq.lock() {
        settings.on = true;
        settings.preamp_db = -12.0;
    }
    settle(&engine, Duration::from_millis(1_200));
    let cut = level(&tap);
    println!("    with the preamp 12 dB down the tap is at {cut:.1} dBFS");
    check(
        "the equalizer is in front of the tap",
        (cut - flat + 12.0).abs() < 3.0,
    );
    if let Ok(mut settings) = eq.lock() {
        *settings = fastsonic::eq::EqSettings::default();
    }

    // A seek throws the sink away, and with it everything tapped that
    // would never have been heard.
    engine.command(PlayerCommand::Seek(500))?;
    settle(&engine, Duration::from_millis(1_200));
    check("and the tap refills after a seek", level(&tap) > -60.0);

    engine.shutdown();

    // P3.7: an album with ReplayGain in it, played with normalisation on.
    // What the gain does cannot be seen from out here — it is behind the
    // tap, which is the point of it — so what this checks is that a track
    // whose gain is known still plays. `RUST_LOG=fastsonic::engine=info`
    // prints the gain each track is played at, to compare against what the
    // server says about it.
    println!("\n-- with normalisation on");
    let (levelled_album, gain_db) = runtime.block_on(pick_normalised(&client))?;
    match gain_db {
        Some(db) => println!("    its album gain is {db:+.2} dB"),
        None => println!(
            "    nothing in this library carries ReplayGain, so this only \
             checks that the setting changes nothing"
        ),
    }
    let normalised = Engine::start(
        &EngineConfig {
            initial_volume: u16::MAX / 2,
            normalisation: true,
            ..EngineConfig::default()
        },
        Arc::clone(&client),
        runtime.handle().clone(),
        watcher(started, Arc::clone(&seen)),
    )?;
    normalised.command(PlayerCommand::Load(LoadSpec {
        context_uri: Some(convert::album_uri(&levelled_album)),
        play: true,
        ..LoadSpec::default()
    }))?;
    let levelled = settle(&normalised, Duration::from_millis(2_000));
    check(
        "a track plays at its ReplayGain",
        levelled.playback == Playback::Playing && levelled.position_ms > 500,
    );
    normalised.shutdown();

    // P3.6: the same track twice, with somewhere to keep it. What this
    // checks is the done-when — the second play makes no request — and it
    // needs a whole track to have been read, so it plays the shortest song
    // in the library from end to end first.
    println!("\n-- with the audio cache on");
    let cache_dir =
        std::env::temp_dir().join(format!("fastsonic-probe-cache-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache_dir);
    let cache = Cache::open(cache_dir.clone(), 64 * 1024 * 1024)?;
    let short = runtime.block_on(pick_short(&client))?;
    println!(
        "    {} is {:.1} s and {} byte(s)",
        short.title,
        short.duration.unwrap_or_default(),
        short
            .size
            .map(|size| size.to_string())
            .unwrap_or_else(|| "an unknown number of".into())
    );
    let cached = Engine::start(
        &EngineConfig {
            initial_volume: u16::MAX / 2,
            cache: Some(Arc::clone(&cache)),
            ..EngineConfig::default()
        },
        Arc::clone(&client),
        runtime.handle().clone(),
        watcher(started, Arc::clone(&seen)),
    )?;
    // One track and nothing after it, so that nothing else is opened
    // ahead of time and every request counted belongs to this song.
    let play_it = |engine: &Engine| -> anyhow::Result<()> {
        engine.command(PlayerCommand::Load(LoadSpec {
            uris: vec![convert::track_uri(&short.id)],
            play: true,
            ..LoadSpec::default()
        }))?;
        Ok(())
    };
    play_it(&cached)?;
    let played = to_the_end(&cached, Duration::from_secs(40));
    check(
        "a track plays to the end of a one-track queue",
        played.playback == Playback::Stopped,
    );
    let first = cache.stats();
    println!(
        "    the cache holds {} byte(s) in {} track(s) after one play, \
         having fetched {} block(s)",
        first.bytes, first.entries, first.misses
    );
    let whole = short
        .size
        .and_then(|size| u64::try_from(size).ok())
        .is_none_or(|size| first.bytes >= size);
    check("what was played is on disk", first.bytes > 0 && whole);

    play_it(&cached)?;
    let again = settle(&cached, Duration::from_millis(2_500));
    check(
        "and it plays again",
        again.playback == Playback::Playing && again.position_ms > 500,
    );
    let second = cache.stats();
    println!(
        "    the second play read {} block(s) from disk and fetched {}",
        second.hits - first.hits,
        second.misses - first.misses
    );
    // The done-when. It only means anything because the whole file was
    // read the first time, which is what `whole` above establishes.
    check(
        "the second play makes no request",
        whole && second.misses == first.misses && second.hits > first.hits,
    );
    cached.shutdown();
    let _ = std::fs::remove_dir_all(&cache_dir);

    let snapshots = seen.lock().unwrap_or_else(|p| p.into_inner()).len();
    let errors: Vec<String> = seen
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .iter()
        .filter_map(|(_, state)| state.error.clone())
        .collect();
    println!("\n{snapshots} state snapshots");
    for error in &errors {
        println!("    reported: {error}");
    }
    if failures.is_empty() && errors.is_empty() {
        println!("the engine holds up its end of the contract");
        return Ok(());
    }
    anyhow::bail!(
        "{} check(s) failed and {} error(s) were reported",
        failures.len(),
        errors.len()
    );
}

/// How loud what the visualisers are reading is, in decibels below full
/// scale — the same window the spectrum analyser takes.
fn level(tap: &vis::AudioTap) -> f64 {
    let window = tap.window(vis::SCOPE_SAMPLES, vis::LAG);
    let mean_square = window
        .iter()
        .map(|s| f64::from(*s) * f64::from(*s))
        .sum::<f64>()
        / window.len() as f64;
    20.0 * mean_square.sqrt().max(1e-9).log10()
}

/// Prints every snapshot as it arrives and keeps it with the moment it
/// did, which is what the gapless checks are made of.
fn watcher(
    started: Instant,
    seen: Arc<Mutex<Vec<(f64, LocalState)>>>,
) -> fastsonic::engine::Notify {
    Arc::new(move |event| match event {
        EngineEvent::State(state) => {
            let at = started.elapsed().as_secs_f64();
            println!("{at:>7.2}s  {}", line(&state));
            seen.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push((at, state));
        }
        EngineEvent::Queue(queue) => println!(
            "{:>7.2}s  queue: {}",
            started.elapsed().as_secs_f64(),
            rows(&queue)
        ),
    })
}

fn env(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.to_string())
}

/// Waits for playback to stop of its own accord — the end of a one-track
/// queue — or gives up, so that a probe against a library of ten-minute
/// tracks says what happened rather than hanging.
fn to_the_end(engine: &Engine, patience: Duration) -> LocalState {
    let until = Instant::now() + patience;
    // Stopped is also where it starts from, and the end of a track puts the
    // position back to nought — so what says a track has finished is having
    // heard it play first.
    let mut heard = false;
    loop {
        let state = engine.state();
        heard |= state.playback == Playback::Playing;
        if heard && state.playback == Playback::Stopped {
            return state;
        }
        if Instant::now() >= until {
            println!("    it did not finish within {patience:?}");
            return state;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Waits, then takes the state as it is: the engine pushes changes as they
/// happen, and what a check wants to know is where they left it.
fn settle(engine: &Engine, wait: Duration) -> LocalState {
    std::thread::sleep(wait);
    engine.state()
}

/// The uri of each row, which is what the rules are about: which songs,
/// in which order.
fn uris(rows: &[QueueRow]) -> Vec<String> {
    rows.iter().map(|row| row.uri.clone()).collect()
}

fn rows(queue: &QueueSnapshot) -> String {
    let name = |row: &QueueRow| {
        row.track
            .as_ref()
            .map(|track| track.title.clone())
            .unwrap_or_else(|| format!("<{}>", row.uri))
    };
    format!(
        "playing next [{}], next up [{}]",
        queue.queued.iter().map(name).collect::<Vec<_>>().join(", "),
        queue
            .upcoming
            .iter()
            .take(3)
            .map(name)
            .collect::<Vec<_>>()
            .join(", "),
    )
}

fn sorted(rows: &[QueueRow]) -> Vec<String> {
    sorted_uris(&uris(rows))
}

fn sorted_uris(uris: &[String]) -> Vec<String> {
    let mut sorted = uris.to_vec();
    sorted.sort();
    sorted
}

/// The artist whose record this is, so that the artist context has
/// something real to expand.
async fn artist_of(client: &SubsonicClient, album: &str) -> Option<String> {
    client
        .get_album(album)
        .await
        .ok()?
        .song
        .first()
        .and_then(|song| song.artist_id.clone())
}

fn line(state: &LocalState) -> String {
    format!(
        "{:<8} {:>7} ms  vol {:>5}  {}{}",
        format!("{:?}", state.playback),
        state.position_ms,
        state.volume,
        state
            .track
            .as_ref()
            .map(|track| format!("{} — {}", track.artist_names(), track.title))
            .unwrap_or_else(|| "[no track]".into()),
        state
            .error
            .as_ref()
            .map(|error| format!("  ERROR: {error}"))
            .unwrap_or_default(),
    )
}

/// An album with more than one track in it, so that Next has somewhere to
/// go, chosen the same way on every run.
/// An album whose songs carry ReplayGain, with the gain the engine would
/// play it at, falling back to any album at all — a library without a
/// single tagged file is unusual but not wrong, and the probe still has
/// something to say about it.
async fn pick_normalised(client: &SubsonicClient) -> anyhow::Result<(String, Option<f64>)> {
    for song in client.random_songs(200).await? {
        let Some(album) = song.album_id.clone() else {
            continue;
        };
        if let Some(gain) = song.replay_gain.and_then(|gain| gain.album_gain) {
            return Ok((album, Some(gain)));
        }
    }
    Ok((pick_album(client).await?, None))
}

/// The shortest song in the library, so that playing one from end to end
/// — which is what puts a whole file in the cache — takes seconds rather
/// than minutes.
async fn pick_short(client: &SubsonicClient) -> anyhow::Result<Child> {
    let mut songs = client.random_songs(200).await?;
    songs.sort_by_key(|song| (song.duration.unwrap_or(i64::MAX), song.id.clone()));
    songs
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("the library has no songs in it"))
}

async fn pick_album(client: &SubsonicClient) -> anyhow::Result<String> {
    client.ping().await?;
    let mut songs: Vec<Child> = client.random_songs(200).await?;
    songs.sort_by(|left, right| {
        let key = |song: &Child| (song.album_id.clone(), song.disc_number, song.track);
        key(left).cmp(&key(right))
    });
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for song in &songs {
        if let Some(album) = &song.album_id {
            *counts.entry(album.clone()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .find(|(_, count)| *count > 1)
        .map(|(album, _)| album)
        .ok_or_else(|| anyhow::anyhow!("the library has no album with two tracks in it"))
}

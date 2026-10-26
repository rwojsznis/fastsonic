//! Who else on the server is listening, from `getNowPlaying`.
//!
//! The answer lists every player that has reported a song as now playing,
//! this one included: Fastsonic reports its own song under `c=fastsonic`
//! (see [`crate::api::subsonic::scrobble`]). The Listening activity panel is
//! about the other people and devices, so [`others`] leaves this app's own
//! entry out.
//!
//! The server never says which entry is this app's. Navidrome's `playerId`
//! is not the player's, and nothing in the answer is unique to one copy of
//! the app, so an entry is taken for this one when all three agree: the
//! signed-in user, this app's client name (or no player name at all), and a
//! song this app played last or is playing now. The same account on another
//! device stays in the list, as does another copy of Fastsonic playing a
//! different song; only a second copy playing the very same song cannot be
//! told apart and is left out with this one.

use crate::api::models::Track;
use crate::api::subsonic::client::CLIENT_NAME;
use crate::api::subsonic::{NowPlayingEntry, convert};

/// How many of this app's latest songs count as its own. The server can
/// still list the last one for a moment after the next one starts.
pub const OWN_SONGS: usize = 2;

/// One player's song, as the panel shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listener {
    pub username: String,
    /// The client the song plays in, as the player reported its name.
    pub player: Option<String>,
    /// Whole minutes since the player last reported the song.
    pub minutes_ago: u32,
    /// Still playing, as far as the server can tell.
    pub playing: bool,
    pub track: Track,
}

impl Listener {
    pub fn from_entry(entry: &NowPlayingEntry) -> Self {
        let track = convert::track(&entry.child);
        let minutes_ago = entry.minutes_ago.unwrap_or_default().max(0) as u32;
        Self {
            username: entry.username.clone().unwrap_or_default(),
            player: entry
                .player_name
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string),
            minutes_ago,
            playing: playing(entry.state.as_deref(), minutes_ago, track.duration_ms),
            track,
        }
    }

    /// Whether this entry is this app's own report: the signed-in user, this
    /// client, and one of the songs it played.
    fn is_this_app(&self, me: &str, own_songs: &[String]) -> bool {
        self.username.eq_ignore_ascii_case(me)
            && self.player.as_deref().is_none_or(names_this_client)
            && self
                .track
                .id
                .as_ref()
                .is_some_and(|id| own_songs.contains(id))
    }
}

/// The `playbackReport` extension says what the player is doing. Without
/// it, a report younger than its song is taken as still playing: Fastsonic
/// repeats its report every 30 seconds, but other clients report a song
/// once, when it starts.
fn playing(state: Option<&str>, minutes_ago: u32, duration_ms: u32) -> bool {
    match state {
        Some(state) => state.eq_ignore_ascii_case("playing"),
        None => minutes_ago == 0 || u64::from(minutes_ago) * 60_000 < u64::from(duration_ms),
    }
}

/// Navidrome names a player after the `c=` it was registered with, and some
/// servers add the user agent in brackets after it.
fn names_this_client(player: &str) -> bool {
    player
        .split([' ', '[', '('])
        .next()
        .is_some_and(|name| name.eq_ignore_ascii_case(CLIENT_NAME))
}

/// Everyone but this app, playing first and then the latest reports first.
/// `me` is the signed-in username and `own_songs` the ids of the songs this
/// app played last.
pub fn others(listeners: Vec<Listener>, me: Option<&str>, own_songs: &[String]) -> Vec<Listener> {
    let mut others: Vec<Listener> = listeners
        .into_iter()
        .filter(|listener| !me.is_some_and(|me| listener.is_this_app(me, own_songs)))
        .collect();
    others.sort_by_key(|listener| (!listener.playing, listener.minutes_ago));
    others
}

/// `Just now`, `12 min ago`, `3 h ago` or `2 d ago`.
pub fn when(minutes_ago: u32) -> String {
    match minutes_ago {
        0 => "Just now".into(),
        1..60 => format!("{minutes_ago} min ago"),
        60..1440 => format!("{} h ago", minutes_ago / 60),
        _ => format!("{} d ago", minutes_ago / 1440),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::subsonic::Child;

    fn entry(user: &str, player: Option<&str>, song: &str, minutes: i64) -> NowPlayingEntry {
        NowPlayingEntry {
            child: Child {
                id: song.into(),
                title: format!("Song {song}"),
                duration: Some(200),
                ..Child::default()
            },
            username: Some(user.into()),
            minutes_ago: Some(minutes),
            player_name: player.map(str::to_string),
            ..NowPlayingEntry::default()
        }
    }

    fn listeners(entries: &[NowPlayingEntry]) -> Vec<Listener> {
        entries.iter().map(Listener::from_entry).collect()
    }

    fn songs(listeners: &[Listener]) -> Vec<(&str, &str)> {
        listeners
            .iter()
            .map(|listener| {
                (
                    listener.username.as_str(),
                    listener.track.id.as_deref().unwrap_or_default(),
                )
            })
            .collect()
    }

    /// This app's own report is left out; the same account on another
    /// client, and other people playing the same song, stay.
    #[test]
    fn this_apps_own_song_is_left_out() {
        let all = listeners(&[
            entry("me", Some("fastsonic"), "s1", 0),
            entry("me", Some("NavidromeUI"), "s1", 0),
            entry("ann", Some("fastsonic"), "s1", 0),
        ]);
        let own = vec!["s1".to_string()];
        assert_eq!(
            songs(&others(all, Some("me"), &own)),
            [("me", "s1"), ("ann", "s1")]
        );
    }

    /// Another copy of Fastsonic under the same account playing something
    /// else is another device, not this one.
    #[test]
    fn another_fastsonic_of_the_same_account_stays() {
        let all = listeners(&[
            entry("me", Some("fastsonic"), "s1", 0),
            entry("me", Some("fastsonic"), "s9", 3),
        ]);
        let own = vec!["s1".to_string()];
        assert_eq!(songs(&others(all, Some("me"), &own)), [("me", "s9")]);
    }

    /// The song before the one playing still counts, since the server can
    /// list it for a moment after the next one starts; a player with no name
    /// is taken for this one; usernames compare without case; and a player
    /// name with the user agent after it is still this client's.
    #[test]
    fn the_last_songs_and_unnamed_players_count_as_this_app() {
        let all = listeners(&[
            entry("Me", None, "s1", 0),
            entry("me", Some("fastsonic [fastsonic/0.13]"), "s0", 0),
            entry("me", Some("fastsonic-ish"), "s0", 0),
        ]);
        let own = vec!["s0".to_string(), "s1".to_string()];
        assert_eq!(songs(&others(all, Some("me"), &own)), [("me", "s0")]);
    }

    /// Before the account is known, or with nothing played here, nobody is
    /// left out.
    #[test]
    fn nothing_is_left_out_without_an_account_or_a_song() {
        let all = listeners(&[entry("me", Some("fastsonic"), "s1", 0)]);
        assert_eq!(others(all.clone(), None, &["s1".into()]).len(), 1);
        assert_eq!(others(all, Some("me"), &[]).len(), 1);
    }

    /// Someone listening now comes first, then the latest reports.
    #[test]
    fn playing_first_then_latest() {
        let mut stale = entry("old", Some("Feishin"), "s3", 30);
        stale.child.duration = Some(60);
        let all = listeners(&[
            stale,
            entry("ann", None, "s1", 2),
            entry("bo", None, "s2", 0),
        ]);
        let shown = others(all, Some("me"), &[]);
        assert_eq!(songs(&shown), [("bo", "s2"), ("ann", "s1"), ("old", "s3")]);
        assert!(shown[0].playing && shown[1].playing && !shown[2].playing);
    }

    /// The `playbackReport` extension's state wins over the age of the
    /// report.
    #[test]
    fn a_reported_state_decides_playing() {
        assert!(playing(Some("playing"), 50, 1000));
        assert!(!playing(Some("paused"), 0, 200_000));
        assert!(playing(None, 3, 200_000));
        assert!(!playing(None, 4, 200_000));
        assert!(playing(None, 0, 0));
    }

    #[test]
    fn when_counts_minutes_hours_and_days() {
        assert_eq!(when(0), "Just now");
        assert_eq!(when(12), "12 min ago");
        assert_eq!(when(125), "2 h ago");
        assert_eq!(when(3000), "2 d ago");
    }

    /// An empty `nowPlaying` object, as servers send when nobody listens,
    /// and an empty player name read as nobody and no player.
    #[test]
    fn empty_answers_read_as_nobody() {
        let empty: crate::api::subsonic::NowPlaying = serde_json::from_str("{}").unwrap();
        assert!(empty.entry.is_empty());
        let listener = Listener::from_entry(&entry("ann", Some("  "), "s1", 0));
        assert_eq!(listener.player, None);
    }
}

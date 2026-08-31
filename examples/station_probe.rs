//! Diagnostic: which context shapes Spotify still resolves for a song
//! radio, run with the stored playback credential.
//!
//!   cargo run --example station_probe -- spotify:track:4uLU6hMCjMI75M1A2tKUQC

use librespot_core::{Session, SessionConfig, cache::Cache};
use librespot_protocol::autoplay_context_request::AutoplayContextRequest;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let track = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "spotify:track:4uLU6hMCjMI75M1A2tKUQC".into());
    let id = track.rsplit(':').next().unwrap_or_default().to_string();

    let dirs = fastpotify::paths::AppDirs::discover();
    let cache = Cache::new(Some(dirs.credentials_dir().as_path()), None, None, None)?;
    let credentials = cache
        .credentials()
        .ok_or_else(|| anyhow::anyhow!("no stored playback credential"))?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let session = Session::new(SessionConfig::default(), Some(cache));
        session.connect(credentials, false).await?;
        println!("connected as {}", session.username());

        let station = format!("spotify:station:track:{id}");
        for uri in [station.as_str(), track.as_str()] {
            print!("get_context({uri}) -> ");
            match session.spclient().get_context(uri).await {
                Ok(ctx) => println!(
                    "ok: uri={:?} pages={} first_page_tracks={}",
                    ctx.uri,
                    ctx.pages.len(),
                    ctx.pages.first().map(|p| p.tracks.len()).unwrap_or(0)
                ),
                Err(error) => println!("ERR: {error}"),
            }
        }
        for uri in [track.as_str(), station.as_str()] {
            let request = AutoplayContextRequest {
                context_uri: Some(uri.to_string()),
                ..Default::default()
            };
            print!("get_autoplay_context({uri}) -> ");
            match session.spclient().get_autoplay_context(&request).await {
                Ok(ctx) => println!(
                    "ok: uri={:?} pages={} first_page_tracks={}",
                    ctx.uri,
                    ctx.pages.len(),
                    ctx.pages.first().map(|p| p.tracks.len()).unwrap_or(0)
                ),
                Err(error) => println!("ERR: {error}"),
            }
        }
        anyhow::Ok(())
    })?;
    Ok(())
}
